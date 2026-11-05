#!/usr/bin/env python3
"""
scripts/zkvm-bench/compare.py
Compare benchmark results between runs

Usage:
  python compare.py <baseline.txt> <current.txt>
  python compare.py profiles/zisk/stats_baseline.txt profiles/zisk/stats_current.txt
"""

import sys
import json
import re
from pathlib import Path
from dataclasses import dataclass, field
from typing import Optional


@dataclass
class ZiskStats:
    steps: int = 0
    total_cost: int = 0
    top_functions: dict[str, int] = field(default_factory=dict)
    cost_distribution: dict[str, dict] = field(default_factory=dict)

    @classmethod
    def from_file(cls, path: Path) -> "ZiskStats":
        content = path.read_text()
        stats = cls()

        # Parse STEPS
        steps_match = re.search(r"STEPS\s+([\d,]+)", content)
        if steps_match:
            stats.steps = int(steps_match.group(1).replace(",", ""))

        # Parse cost distribution (BASE, MAIN, OPCODES, PRECOMPILES, MEMORY)
        for match in re.finditer(
            r"^(\w+)\s+([\d,]+)\s+([\d.]+)%", content, re.MULTILINE
        ):
            category = match.group(1).upper()
            if category in ["BASE", "MAIN", "OPCODES", "PRECOMPILES", "MEMORY"]:
                stats.cost_distribution[category.lower()] = {
                    "cost": int(match.group(2).replace(",", "")),
                    "percent": float(match.group(3)),
                }

        # Parse top cost functions
        func_pattern = r"^\s*([\d,]+)\s+([\d.]+)%\s+(.+)$"
        in_top_cost = False
        for line in content.split("\n"):
            if "TOP COST FUNCTIONS" in line:
                in_top_cost = True
                continue
            if in_top_cost:
                match = re.match(func_pattern, line.strip())
                if match:
                    cost = int(match.group(1).replace(",", ""))
                    func_name = match.group(3).strip()
                    stats.top_functions[func_name] = cost
                elif line.strip() and not line.startswith("-") and stats.top_functions:
                    break

        stats.total_cost = sum(stats.top_functions.values())
        return stats


def format_number(n: int) -> str:
    """Format number with thousand separators."""
    return f"{n:,}"


def format_change(base: int, curr: int) -> str:
    """Format percentage change."""
    if base == 0:
        return "NEW" if curr > 0 else "N/A"
    change = ((curr - base) / base) * 100
    return f"{change:+.2f}%"


def print_separator(char: str = "-", width: int = 95):
    print(char * width)


def compare(baseline_path: Path, current_path: Path):
    """Compare two ZisK stats files."""
    if not baseline_path.exists():
        print(f"Error: Baseline file not found: {baseline_path}")
        sys.exit(1)
    if not current_path.exists():
        print(f"Error: Current file not found: {current_path}")
        sys.exit(1)

    base = ZiskStats.from_file(baseline_path)
    curr = ZiskStats.from_file(current_path)

    # Header
    print()
    print("=" * 95)
    print(f"{'zkVM Benchmark Comparison':^95}")
    print("=" * 95)
    print(f"Baseline: {baseline_path.name}")
    print(f"Current:  {current_path.name}")
    print()

    # Summary metrics
    print_separator("=")
    print(f"{'Summary Metrics':^95}")
    print_separator("=")
    print(f"{'Metric':<40} {'Baseline':>18} {'Current':>18} {'Change':>15}")
    print_separator()

    # Total steps
    step_change = format_change(base.steps, curr.steps)
    improvement = "improved" if curr.steps < base.steps else "regressed" if curr.steps > base.steps else "unchanged"
    print(
        f"{'Total Steps':<40} {format_number(base.steps):>18} {format_number(curr.steps):>18} {step_change:>15}"
    )

    # Cost distribution comparison
    if base.cost_distribution and curr.cost_distribution:
        print()
        print_separator("=")
        print(f"{'Cost Distribution':^95}")
        print_separator("=")
        print(f"{'Category':<40} {'Baseline':>18} {'Current':>18} {'Change':>15}")
        print_separator()

        categories = set(base.cost_distribution.keys()) | set(
            curr.cost_distribution.keys()
        )
        for cat in sorted(categories):
            base_cost = base.cost_distribution.get(cat, {}).get("cost", 0)
            curr_cost = curr.cost_distribution.get(cat, {}).get("cost", 0)
            change = format_change(base_cost, curr_cost)
            print(
                f"{cat.upper():<40} {format_number(base_cost):>18} {format_number(curr_cost):>18} {change:>15}"
            )

    # Top functions comparison
    print()
    print_separator("=")
    print(f"{'Top Functions by Cost (Current)':^95}")
    print_separator("=")
    print(f"{'Function':<55} {'Baseline':>12} {'Current':>12} {'Change':>12}")
    print_separator()

    all_funcs = set(base.top_functions.keys()) | set(curr.top_functions.keys())
    sorted_funcs = sorted(
        all_funcs, key=lambda f: curr.top_functions.get(f, 0), reverse=True
    )

    for func in sorted_funcs[:15]:
        base_cost = base.top_functions.get(func, 0)
        curr_cost = curr.top_functions.get(func, 0)
        change = format_change(base_cost, curr_cost)

        # Truncate function name for display
        display_name = func[:53] + ".." if len(func) > 55 else func
        print(
            f"{display_name:<55} {format_number(base_cost):>12} {format_number(curr_cost):>12} {change:>12}"
        )

    # New/removed functions
    new_funcs = [
        f
        for f in curr.top_functions
        if f not in base.top_functions and curr.top_functions[f] > 0
    ]
    removed_funcs = [
        f
        for f in base.top_functions
        if f not in curr.top_functions and base.top_functions[f] > 0
    ]

    if new_funcs:
        print()
        print("New functions in current:")
        for func in sorted(new_funcs, key=lambda f: curr.top_functions[f], reverse=True)[:5]:
            cost = curr.top_functions[func]
            display_name = func[:70] + ".." if len(func) > 72 else func
            print(f"  + {display_name}: {format_number(cost)}")

    if removed_funcs:
        print()
        print("Functions removed/optimized away:")
        for func in sorted(removed_funcs, key=lambda f: base.top_functions[f], reverse=True)[:5]:
            cost = base.top_functions[func]
            display_name = func[:70] + ".." if len(func) > 72 else func
            print(f"  - {display_name}: {format_number(cost)}")

    # Final summary
    print()
    print_separator("=")
    if curr.steps < base.steps:
        diff = base.steps - curr.steps
        pct = (diff / base.steps) * 100
        print(f"IMPROVEMENT: {format_number(diff)} fewer steps ({pct:.2f}% reduction)")
    elif curr.steps > base.steps:
        diff = curr.steps - base.steps
        pct = (diff / base.steps) * 100
        print(f"REGRESSION: {format_number(diff)} more steps ({pct:.2f}% increase)")
    else:
        print("NO CHANGE in total steps")
    print_separator("=")
    print()


def main():
    if len(sys.argv) == 2 and sys.argv[1] in ["-h", "--help"]:
        print(__doc__)
        sys.exit(0)

    if len(sys.argv) != 3:
        print("Usage: compare.py <baseline.txt> <current.txt>")
        print()
        print("Compare two ZisK stats files to analyze performance changes.")
        print()
        print("Example:")
        print("  python compare.py profiles/zisk/stats_baseline.txt profiles/zisk/stats_current.txt")
        sys.exit(1)

    compare(Path(sys.argv[1]), Path(sys.argv[2]))


if __name__ == "__main__":
    main()
