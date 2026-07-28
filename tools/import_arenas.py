#!/usr/bin/env python3
"""
Import Screeps Arena seasons and arenas from a mitmproxy .flows file.

Scans the flow file for successful responses from:
  GET /api/season/list            - all seasons (historical + current)
  GET /api/season/current         - which season is currently active
  GET /api/season/<id>/arenas     - all arenas belonging to a season

Upserts discovered seasons and arenas into the local database at
~/.local/share/screeps_arena_sim/arenas.json (respecting $XDG_DATA_HOME).

Usage:
    python3 import_arenas.py <path-to-session.flows> [--db <path>]
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


def get_default_db_path() -> Path:
    xdg_data = os.environ.get("XDG_DATA_HOME", "")
    if xdg_data:
        return Path(xdg_data) / "screeps_arena_sim" / "arenas.json"
    home = os.environ.get("HOME", "")
    if home:
        return Path(home) / ".local" / "share" / "screeps_arena_sim" / "arenas.json"
    return Path("./arenas.json")


def load_db(db_path: Path) -> dict:
    if db_path.exists():
        with open(db_path) as f:
            return json.load(f)
    return {"seasons": [], "arenas": []}


def save_db(db_path: Path, db: dict) -> None:
    db_path.parent.mkdir(parents=True, exist_ok=True)
    with open(db_path, "w") as f:
        json.dump(db, f, indent=2)


def parse_flows(flows_path: Path) -> tuple[list[dict], str | None, list[dict]]:
    """
    Read the flow file and extract season and arena data.

    Returns:
        seasons:        list of season records from /api/season/list
        current_id:     season _id from /api/season/current, or None
        arenas:         list of arena records from /api/season/<id>/arenas
    """
    seasons = []
    current_id = None
    arenas = []

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

            if body.get("ok") != 1:
                continue

            path = flow.request.path.split("?")[0].rstrip("/")
            parts = path.strip("/").split("/")  # ['api', 'season', ...]

            if parts[:2] != ["api", "season"]:
                continue

            if len(parts) == 3 and parts[2] == "list":
                # /api/season/list
                for s in body.get("seasons", []):
                    seasons.append({
                        "id": s["_id"],
                        "name": s.get("name", ""),
                        "active": s.get("active", False),
                        "start_date": s.get("startDate", ""),
                        "end_date": s.get("endDate", ""),
                    })

            elif len(parts) == 3 and parts[2] == "current":
                # /api/season/current
                s = body.get("season", {})
                current_id = s.get("_id")

            elif len(parts) == 4 and parts[3] == "arenas":
                # /api/season/<id>/arenas
                season_id = parts[2]
                for a in body.get("arenas", []):
                    arenas.append({
                        "id": a["_id"],
                        "season_id": season_id,
                        "name": a.get("name", ""),
                        "advanced": a.get("advanced", False),
                        "folder_name": a.get("folderName", ""),
                        "active": a.get("active", True),
                    })

    return seasons, current_id, arenas


def upsert(collection: list[dict], incoming: list[dict], key: str = "id") -> tuple[int, int]:
    """Upsert incoming records into collection by key. Returns (added, updated)."""
    index = {item[key]: i for i, item in enumerate(collection)}
    added = updated = 0
    for record in incoming:
        k = record[key]
        if k in index:
            if collection[index[k]] != record:
                collection[index[k]] = record
                updated += 1
        else:
            collection.append(record)
            index[k] = len(collection) - 1
            added += 1
    return added, updated


def main() -> None:
    parser = argparse.ArgumentParser(description="Import seasons and arenas from a mitmproxy .flows file")
    parser.add_argument("flows", type=Path, help="Path to the .flows file")
    parser.add_argument("--db", type=Path, default=None, help="Path to the arena database JSON (default: XDG data dir)")
    args = parser.parse_args()

    if not args.flows.exists():
        print(f"Error: flows file not found: {args.flows}", file=sys.stderr)
        sys.exit(1)

    db_path = args.db or get_default_db_path()
    db = load_db(db_path)
    if "seasons" not in db:
        db["seasons"] = []

    seasons, current_id, arenas = parse_flows(args.flows)

    # Mark the active season from /api/season/current if we found it,
    # overriding whatever the season list reports (they should agree, but
    # /api/season/current is authoritative for "which is current right now").
    if current_id:
        for s in seasons:
            s["active"] = (s["id"] == current_id)

    seasons_added, seasons_updated = upsert(db["seasons"], seasons)
    arenas_added, arenas_updated = upsert(db["arenas"], arenas)

    save_db(db_path, db)

    current_season = next((s for s in db["seasons"] if s.get("active")), None)

    print(f"Seasons: {seasons_added} added, {seasons_updated} updated")
    print(f"Arenas:  {arenas_added} added, {arenas_updated} updated")
    print(f"Database: {db_path}")
    if current_season:
        print(f"Current season: {current_season['name']} ({current_season['id']})")
    print()

    # Print all seasons with their arenas grouped beneath them
    season_index = {s["id"]: s for s in db["seasons"]}
    arenas_by_season: dict[str, list] = {}
    for a in db["arenas"]:
        arenas_by_season.setdefault(a["season_id"], []).append(a)

    for season in db["seasons"]:
        marker = " [CURRENT]" if season.get("active") else ""
        print(f"{season['name']}{marker}  ({season['id']})")
        for a in sorted(arenas_by_season.get(season["id"], []), key=lambda x: (x["advanced"], x["name"])):
            adv = " [advanced]" if a["advanced"] else ""
            print(f"  {a['name']}{adv}  folder={a['folder_name']}  ({a['id']})")
        print()


if __name__ == "__main__":
    main()
