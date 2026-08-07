#!/usr/bin/env python3
"""
Extract replay files from a mitmproxy .flows file.

For each game found in the flow file, aggregates tick snapshots from all captured
GET /api/game/<game_id>/replay/<tick> endpoints, orders them by `gameTime`,
deduplicates ticks, verifies tick continuity from tick 0 to max tick, and saves as:
  replay-<game_id>.json

Usage:
    python3 extract_replay.py <path-to-session.flows> [--output-dir <path>]
"""

import argparse
import json
import sys
from pathlib import Path

try:
    from mitmproxy import io as mio
    from mitmproxy.http import HTTPFlow
except ImportError:
    print("Error: mitmproxy Python library not found. Install with: pip install mitmproxy", file=sys.stderr)
    sys.exit(1)


def parse_replays_from_flows(flows_path: Path) -> dict[str, dict[int, dict]]:
    """
    Scan the flow file and collect replay ticks for each game ID.
    
    Returns:
        game_ticks: dict[game_id, dict[gameTime, tick_object]]
    """
    game_ticks: dict[str, dict[int, dict]] = {}

    with open(flows_path, "rb") as f:
        for flow in mio.FlowReader(f).stream():
            if not isinstance(flow, HTTPFlow):
                continue
            if flow.response is None or flow.response.status_code != 200:
                continue

            path = flow.request.path.split("?")[0].rstrip("/")
            parts = path.strip("/").split("/")  # ['api', 'game', <id>, 'replay', <chunk>]

            if len(parts) == 5 and parts[:2] == ["api", "game"] and parts[3] == "replay":
                game_id = parts[2]

                try:
                    body = json.loads(flow.response.get_content())
                except (json.JSONDecodeError, UnicodeDecodeError):
                    continue

                if not isinstance(body, list):
                    continue

                if game_id not in game_ticks:
                    game_ticks[game_id] = {}

                for tick_snapshot in body:
                    if isinstance(tick_snapshot, dict) and "gameTime" in tick_snapshot:
                        gt = int(tick_snapshot["gameTime"])
                        game_ticks[game_id][gt] = tick_snapshot

    return game_ticks


def check_completeness(ticks_map: dict[int, dict]) -> tuple[bool, str]:
    """
    Checks whether all ticks from 0 to max_tick are present without gaps.
    """
    if not ticks_map:
        return False, "No ticks found"

    sorted_times = sorted(ticks_map.keys())
    min_tick = sorted_times[0]
    max_tick = sorted_times[-1]
    total_ticks = len(sorted_times)
    expected_count = max_tick - min_tick + 1

    missing = []
    for expected in range(min_tick, max_tick + 1):
        if expected not in ticks_map:
            missing.append(expected)

    if min_tick != 0:
        return False, f"Incomplete: Missing initial tick 0 (starts at tick {min_tick}, max tick {max_tick}, captured {total_ticks} ticks)"

    if missing:
        missing_preview = f"{missing[:5]}..." if len(missing) > 5 else f"{missing}"
        return False, f"Incomplete: Missing {len(missing)} tick(s) between tick 0 and {max_tick} (missing ticks: {missing_preview})"

    return True, f"Complete: All {total_ticks} ticks (0 through {max_tick}) present with no gaps"


def extract_replays(flows_path: Path, output_dir: Path) -> None:
    print(f"Scanning flow file: {flows_path} ...")
    game_ticks = parse_replays_from_flows(flows_path)

    if not game_ticks:
        print("No replay data found in flow file.")
        return

    output_dir.mkdir(parents=True, exist_ok=True)

    for game_id, ticks_map in game_ticks.items():
        is_complete, status_msg = check_completeness(ticks_map)
        sorted_ticks = [ticks_map[gt] for gt in sorted(ticks_map.keys())]

        out_filename = f"replay-{game_id}.json"
        out_path = output_dir / out_filename

        with open(out_path, "w") as f:
            json.dump(sorted_ticks, f, indent=2)

        if is_complete:
            print(f"✓ Extracted {out_filename}: {status_msg}")
        else:
            print(f"⚠️  Extracted {out_filename}: {status_msg}", file=sys.stderr)


def main() -> None:
    parser = argparse.ArgumentParser(description="Extract replay files from a mitmproxy .flows file")
    parser.add_argument("flows", type=Path, help="Path to the .flows file")
    parser.add_argument("--output-dir", type=Path, default=Path("."),
                        help="Directory where replay-<game_id>.json files will be saved (default: current directory)")
    args = parser.parse_args()

    if not args.flows.exists():
        print(f"Error: flows file not found: {args.flows}", file=sys.stderr)
        sys.exit(1)

    extract_replays(args.flows, args.output_dir)


if __name__ == "__main__":
    main()
