#!/usr/bin/env python3
"""
Import map layout data (terrain + initial objects) from a mitmproxy .flows file.

For each game found in the flow file, looks for a matching pair of:
  GET /api/game/<game_id>               - provides arena_id and terrain string
  GET /api/game/<game_id>/replay/100    - provides tick 1 object list

Layouts are saved under the XDG data directory as:
  layouts/<arena_id>/<game_id>/terrain.json
  layouts/<arena_id>/<game_id>/objects.json

The tick 1 object snapshot is used as the closest available approximation to
the initial game state. Creeps that are mid-spawn at tick 1 are excluded (their
_id appears in the spawn's `spawning.id` field) since they did not exist at
tick 0.  Creeps that were already on the map at tick 0 and moved during tick 1
will have incorrect positions — this is an unavoidable limitation of using the
replay API.

Usage:
    python3 import_layouts.py <path-to-session.flows> [--data-dir <path>]
"""

import argparse
import json
import os
import sys
from pathlib import Path

try:
    from mitmproxy import io as mio
    from mitmproxy.http import HTTPFlow
except ImportError:
    print("Error: mitmproxy Python library not found. Install with: pip install mitmproxy", file=sys.stderr)
    sys.exit(1)


def get_default_data_dir() -> Path:
    xdg_data = os.environ.get("XDG_DATA_HOME", "")
    if xdg_data:
        return Path(xdg_data) / "screeps_arena_sim"
    home = os.environ.get("HOME", "")
    if home:
        return Path(home) / ".local" / "share" / "screeps_arena_sim"
    return Path(".")


def parse_flows(flows_path: Path) -> tuple[dict[str, dict], dict[str, list]]:
    """
    Scan the flow file and collect:
      game_info[game_id]  = {"arena_id": ..., "terrain": ...}
      tick1_objects[game_id] = [list of tick-1 objects]

    Returns both dicts; only game_ids present in both are complete layouts.
    """
    game_info: dict[str, dict] = {}
    tick1_objects: dict[str, list] = {}

    with open(flows_path, "rb") as f:
        for flow in mio.FlowReader(f).stream():
            if not isinstance(flow, HTTPFlow):
                continue
            if flow.response is None or flow.response.status_code != 200:
                continue

            try:
                body = json.loads(flow.response.get_content())
            except (json.JSONDecodeError, UnicodeDecodeError):
                continue

            if not isinstance(body, list) and body.get("ok") != 1:
                continue

            path = flow.request.path.split("?")[0].rstrip("/")
            parts = path.strip("/").split("/")  # ['api', 'game', <id>, ...]

            if parts[:2] != ["api", "game"] or len(parts) < 3:
                continue

            game_id = parts[2]

            if len(parts) == 3:
                # GET /api/game/<game_id>
                game = body.get("game", {}).get("game", {})
                arena_id = body.get("game", {}).get("arena")
                terrain = game.get("terrain")
                if arena_id and terrain:
                    game_info[game_id] = {
                        "arena_id": arena_id,
                        "terrain": terrain,
                    }

            elif len(parts) == 4 and parts[3] == "replay":
                # This shouldn't match /replay/100 since that has 5 parts — skip
                pass

            elif len(parts) == 5 and parts[3] == "replay":
                # GET /api/game/<game_id>/replay/<tick>
                # We only want tick 1 (replay/100 returns tick 1 objects)
                tick_param = parts[4]
                if tick_param != "100":
                    continue

                # The replay endpoint returns a raw JSON array of tick snapshots
                ticks = body if isinstance(body, list) else []
                if not ticks:
                    continue

                tick1 = ticks[0]
                objects = tick1.get("objects", [])
                tick1_objects[game_id] = objects

    return game_info, tick1_objects


def filter_objects(objects: list[dict]) -> list[dict]:
    """
    Remove mid-spawn creeps from the tick 1 object list.

    Any spawn with a `spawning` field has a creep currently being produced.
    That creep's _id is listed in spawning.id — we exclude the creep because it did
    not exist as a real creep at tick 0. We also strip the `spawning` dict from the spawn itself
    so the spawn is not discarded.
    """
    spawning_ids = set()
    for obj in objects:
        spawning = obj.get("spawning")
        if isinstance(spawning, dict) and "id" in spawning:
            spawning_ids.add(str(spawning["id"]))

    filtered = []
    for obj in objects:
        obj_id = str(obj.get("_id")) if "_id" in obj else None
        if obj_id in spawning_ids:
            continue

        spawning = obj.get("spawning")
        obj_type = obj.get("type") or obj.get("prototypeName")
        
        # Creep being spawned
        if obj_type in ("creep", "Creep") and (spawning is True or isinstance(spawning, dict)):
            continue

        # If it's a spawn currently spawning a creep, keep the spawn structure but remove the spawning key
        if obj_type in ("spawn", "StructureSpawn") and isinstance(spawning, dict):
            obj_copy = dict(obj)
            del obj_copy["spawning"]
            filtered.append(obj_copy)
            continue

        filtered.append(obj)

    return filtered


def save_layout(data_dir: Path, arena_id: str, game_id: str,
                terrain: str, objects: list[dict]) -> Path:
    layout_dir = data_dir / "layouts" / arena_id / game_id
    layout_dir.mkdir(parents=True, exist_ok=True)

    terrain_path = layout_dir / "terrain.json"
    objects_path = layout_dir / "objects.json"

    with open(terrain_path, "w") as f:
        json.dump({"terrain": terrain}, f)

    with open(objects_path, "w") as f:
        json.dump(objects, f, indent=2)

    return layout_dir


def main() -> None:
    parser = argparse.ArgumentParser(description="Import map layouts from a mitmproxy .flows file")
    parser.add_argument("flows", type=Path, help="Path to the .flows file")
    parser.add_argument("--data-dir", type=Path, default=None,
                        help="Path to the data directory (default: XDG data dir)")
    args = parser.parse_args()

    if not args.flows.exists():
        print(f"Error: flows file not found: {args.flows}", file=sys.stderr)
        sys.exit(1)

    data_dir = args.data_dir or get_default_data_dir()

    print(f"Scanning {args.flows} ...")
    game_info, tick1_objects = parse_flows(args.flows)

    complete = set(game_info) & set(tick1_objects)
    missing_replay = set(game_info) - set(tick1_objects)
    missing_game = set(tick1_objects) - set(game_info)

    if not complete:
        print("No complete game+replay pairs found.")
        if missing_replay:
            print(f"  {len(missing_replay)} game(s) with no matching replay/100 response")
        if missing_game:
            print(f"  {len(missing_game)} replay(s) with no matching game response")
        return

    saved = 0
    skipped = 0
    for game_id in sorted(complete):
        info = game_info[game_id]
        arena_id = info["arena_id"]
        terrain = info["terrain"]
        objects = filter_objects(tick1_objects[game_id])

        layout_dir = data_dir / "layouts" / arena_id / game_id
        if layout_dir.exists():
            skipped += 1
            continue

        save_layout(data_dir, arena_id, game_id, terrain, objects)
        print(f"  Saved layout: arena={arena_id}  game={game_id}  ({len(objects)} objects)")
        saved += 1

    print(f"\n{saved} layout(s) saved, {skipped} already present (skipped).")
    if missing_replay:
        print(f"{len(missing_replay)} game(s) skipped: no replay/100 response captured.")
    print(f"Data directory: {data_dir / 'layouts'}")


if __name__ == "__main__":
    main()
