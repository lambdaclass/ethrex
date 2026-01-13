#!/usr/bin/env python3
import argparse
import csv
import math
import sys
from typing import Dict, Tuple


def parse_csv(path: str, column: str) -> Dict[str, float]:
    values: Dict[str, float] = {}
    with open(path, "r", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        if column not in reader.fieldnames:
            raise ValueError(f"column '{column}' not found in {path}")
        for row in reader:
            name = row.get("fixture")
            if not name:
                continue
            raw = (row.get(column) or "").strip()
            if raw == "":
                continue
            try:
                values[name] = float(raw)
            except ValueError:
                continue
    return values


def pct_change(base: float, comp: float) -> float:
    if base == 0.0:
        return math.inf if comp != 0.0 else 0.0
    return ((comp - base) / base) * 100.0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Compare two opcode bench CSVs and report percent change."
    )
    parser.add_argument("baseline", help="path to baseline CSV")
    parser.add_argument("comparison", help="path to comparison CSV")
    parser.add_argument(
        "--column",
        default="adjusted_ns",
        help="column to compare (default: adjusted_ns)",
    )
    parser.add_argument(
        "--fallback-column",
        default="avg_ns_per_iter",
        help="fallback column if primary is empty for a fixture",
    )
    args = parser.parse_args()

    baseline = parse_csv(args.baseline, args.column)
    comparison = parse_csv(args.comparison, args.column)

    if args.fallback_column and args.fallback_column != args.column:
        baseline_fallback = parse_csv(args.baseline, args.fallback_column)
        comparison_fallback = parse_csv(args.comparison, args.fallback_column)
    else:
        baseline_fallback = {}
        comparison_fallback = {}

    names = sorted(set(baseline) | set(comparison) | set(baseline_fallback) | set(comparison_fallback))

    print(
        "{:<28} {:>14} {:>14} {:>12}".format(
            "fixture", "baseline", "comparison", "% change"
        )
    )
    print("-" * 70)

    missing = 0
    for name in names:
        base = baseline.get(name, baseline_fallback.get(name))
        comp = comparison.get(name, comparison_fallback.get(name))
        if base is None or comp is None:
            missing += 1
            continue
        change = pct_change(base, comp)
        change_str = "inf" if math.isinf(change) else f"{change:>11.2f}%"
        print(f"{name:<28} {base:>14.2f} {comp:>14.2f} {change_str}")

    if missing:
        print(f"\nSkipped {missing} fixture(s) missing data in either CSV", file=sys.stderr)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
