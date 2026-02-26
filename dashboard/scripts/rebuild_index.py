#!/usr/bin/env python3
"""Scan a data directory for benchmark JSON files and generate index.json.

Usage:
    python3 rebuild_index.py [--data-dir DATA_DIR] [--output OUTPUT]
"""

import argparse
import json
import os
import re
from pathlib import Path


# Pattern: <date>/<commit>-bench.json
BENCH_PATTERN = re.compile(r"^(\d{4}-\d{2}-\d{2})/([a-f0-9]+)-bench\.json$")


def scan_data_dir(data_dir: str) -> list[dict]:
    """Scan data_dir for benchmark files and return sorted index entries."""
    entries: list[dict] = []
    base = Path(data_dir)

    if not base.exists():
        return entries

    for date_dir in sorted(base.iterdir()):
        if not date_dir.is_dir():
            continue

        date_name = date_dir.name

        # Find all *-bench.json files (primary key)
        for bench_file in sorted(date_dir.glob("*-bench.json")):
            name = bench_file.name
            # Skip jit-bench files
            if name.endswith("-jit-bench.json"):
                continue

            commit = name.removesuffix("-bench.json")
            rel_bench = f"{date_name}/{name}"

            entry: dict = {
                "date": date_name,
                "commit": commit,
                "bench": rel_bench,
            }

            # Check for optional companion files
            jit_bench = date_dir / f"{commit}-jit-bench.json"
            if jit_bench.exists():
                entry["jit_bench"] = f"{date_name}/{commit}-jit-bench.json"

            regression = date_dir / f"{commit}-regression.json"
            if regression.exists():
                entry["regression"] = f"{date_name}/{commit}-regression.json"

            jit_regression = date_dir / f"{commit}-jit-regression.json"
            if jit_regression.exists():
                entry["jit_regression"] = f"{date_name}/{commit}-jit-regression.json"

            cross_client = date_dir / f"{commit}-cross-client.json"
            if cross_client.exists():
                entry["cross_client"] = f"{date_name}/{commit}-cross-client.json"

            entries.append(entry)

    return entries


def write_index(runs: list[dict], output_path: str) -> None:
    """Write the index.json file."""
    index = {"runs": runs}
    try:
        os.makedirs(os.path.dirname(output_path) or ".", exist_ok=True)
        with open(output_path, "w") as f:
            json.dump(index, f, indent=2, sort_keys=False)
            f.write("\n")
    except OSError as e:
        raise SystemExit(f"Error writing {output_path}: {e}") from e


def main():
    parser = argparse.ArgumentParser(description="Rebuild dashboard index.json")
    parser.add_argument(
        "--data-dir",
        default="data",
        help="Directory containing date-stamped benchmark data",
    )
    parser.add_argument(
        "--output",
        default="data/index.json",
        help="Output path for index.json",
    )
    args = parser.parse_args()

    runs = scan_data_dir(args.data_dir)
    write_index(runs, args.output)
    print(f"Wrote {len(runs)} entries to {args.output}")


if __name__ == "__main__":
    main()
