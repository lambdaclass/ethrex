#!/usr/bin/env python3
"""
Snap Sync Benchmark Tool

Parse ethrex sync logs to extract performance metrics and compare across branches.

Usage:
    # Parse a single run
    python3 sync_benchmark.py parse <log_file> [--output results.json]

    # Parse all runs in a directory
    python3 sync_benchmark.py parse-all <logs_dir> [--output benchmark_results.json]

    # Compare results
    python3 sync_benchmark.py compare <results.json> [--format table|csv|markdown]

    # Compare two specific runs
    python3 sync_benchmark.py diff <run1_id> <run2_id> --results <results.json>

    # Parse with server prefix (for cross-server comparison)
    python3 sync_benchmark.py parse-all <logs_dir> --prefix server1 --output server1_results.json

    # Merge results from multiple servers
    python3 sync_benchmark.py merge server1_results.json server2_results.json --output combined.json
"""

import argparse
import json
import os
import re
import sys
from dataclasses import dataclass, field, asdict
from datetime import datetime
from pathlib import Path
from typing import Optional


# Phase patterns
PHASE_PATTERNS = {
    "block_headers": r"PHASE 1/8: BLOCK HEADERS",
    "account_ranges": r"PHASE 2/8: ACCOUNT RANGES",
    "account_insertion": r"PHASE 3/8: ACCOUNT INSERTION",
    "storage_ranges": r"PHASE 4/8: STORAGE RANGES",
    "storage_insertion": r"PHASE 5/8: STORAGE INSERTION",
    "state_healing": r"PHASE 6/8: STATE HEALING",
    "storage_healing": r"PHASE 7/8: STORAGE HEALING",
    "bytecodes": r"PHASE 8/8: BYTECODES",
}

# Completion patterns
COMPLETION_PATTERNS = {
    "block_headers": r"✓ BLOCK HEADERS complete: ([\d,]+) headers in (\d+:\d+:\d+)",
    "account_ranges": r"✓ ACCOUNT RANGES complete: ([\d,]+) accounts in (\d+:\d+:\d+)",
    "account_insertion": r"✓ ACCOUNT INSERTION complete: ([\d,]+) accounts inserted in (\d+:\d+:\d+)",
    "storage_ranges": r"✓ STORAGE RANGES complete: ([\d,]+) storage slots in (\d+:\d+:\d+)",
    "storage_insertion": r"✓ STORAGE INSERTION complete: ([\d,]+) storage slots inserted in (\d+:\d+:\d+)",
    "state_healing": r"✓ STATE HEALING complete: ([\d,]+) state paths healed in (\d+:\d+:\d+)",
    "storage_healing": r"✓ STORAGE HEALING complete: ([\d,]+) storage accounts healed in (\d+:\d+:\d+)",
    "bytecodes": r"✓ BYTECODES complete: ([\d,]+) bytecodes in (\d+:\d+:\d+)",
}

# Rate patterns
RATE_PATTERNS = {
    "headers": r"Rate: ([\d,]+) headers/s",
    "accounts": r"Rate: ([\d,]+) accounts/s",
    "storage": r"Rate: ([\d,]+) slots/s",
    "bytecodes": r"Rate: ([\d,]+) bytecodes/s",
}

# Summary file pattern
SUMMARY_PATTERN = r"Run #(\d+).*ID: (\d+_\d+).*Branch:\s+(\S+).*Commit:\s+(\S+).*Result:\s+(\S+)"


@dataclass
class PhaseMetrics:
    """Metrics for a single sync phase."""
    name: str
    count: int = 0
    duration_secs: float = 0.0
    avg_rate: float = 0.0
    peak_rate: float = 0.0
    rates: list = field(default_factory=list)  # Time series of rates

    def to_dict(self):
        return {
            "name": self.name,
            "count": self.count,
            "duration_secs": self.duration_secs,
            "avg_rate": self.avg_rate,
            "peak_rate": self.peak_rate,
        }


@dataclass
class SyncMetrics:
    """Complete metrics for a sync run."""
    run_id: str
    branch: str
    commit: str
    network: str
    start_time: str
    total_time_secs: float = 0.0
    success: bool = False
    target_block: int = 0
    phases: dict = field(default_factory=dict)
    errors: list = field(default_factory=list)

    def to_dict(self):
        return {
            "run_id": self.run_id,
            "branch": self.branch,
            "commit": self.commit,
            "network": self.network,
            "start_time": self.start_time,
            "total_time_secs": self.total_time_secs,
            "success": self.success,
            "target_block": self.target_block,
            "phases": {k: v.to_dict() for k, v in self.phases.items()},
            "errors": self.errors,
        }


def parse_time(time_str: str) -> float:
    """Parse HH:MM:SS format to seconds."""
    parts = time_str.split(":")
    if len(parts) == 3:
        h, m, s = map(int, parts)
        return h * 3600 + m * 60 + s
    elif len(parts) == 2:
        m, s = map(int, parts)
        return m * 60 + s
    return 0


def parse_number(num_str: str) -> int:
    """Parse number string with commas."""
    return int(num_str.replace(",", ""))


def extract_timestamp(line: str) -> Optional[str]:
    """Extract timestamp from log line."""
    match = re.match(r"(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})", line)
    return match.group(1) if match else None


def parse_log_file(log_path: Path, network: str = "unknown") -> Optional[SyncMetrics]:
    """Parse a single ethrex log file and extract metrics."""
    if not log_path.exists():
        print(f"Warning: Log file not found: {log_path}")
        return None

    content = log_path.read_text(errors='replace')
    lines = content.split('\n')

    # Initialize metrics
    metrics = SyncMetrics(
        run_id="",
        branch="unknown",
        commit="unknown",
        network=network,
        start_time="",
    )

    # Extract version info (branch, commit)
    version_match = re.search(r"ethrex version: ethrex/v[\d.]+-([^/]+)-([a-f0-9]+)/", content)
    if version_match:
        metrics.branch = version_match.group(1)
        metrics.commit = version_match.group(2)

    # Extract start time
    if lines:
        ts = extract_timestamp(lines[0])
        if ts:
            metrics.start_time = ts

    # Extract target block
    target_match = re.search(r"Target:.*block #([\d,]+)", content)
    if target_match:
        metrics.target_block = parse_number(target_match.group(1))

    # Track phase completions and aggregate durations
    phase_completions = {}  # phase_name -> list of (count, duration_secs)

    for phase_name, pattern in COMPLETION_PATTERNS.items():
        for match in re.finditer(pattern, content):
            count = parse_number(match.group(1))
            duration = parse_time(match.group(2))
            if phase_name not in phase_completions:
                phase_completions[phase_name] = []
            phase_completions[phase_name].append((count, duration))

    # Create phase metrics with aggregated data
    for phase_name, completions in phase_completions.items():
        total_count = sum(c[0] for c in completions)
        total_duration = sum(c[1] for c in completions)
        avg_rate = total_count / total_duration if total_duration > 0 else 0

        metrics.phases[phase_name] = PhaseMetrics(
            name=phase_name,
            count=total_count,
            duration_secs=total_duration,
            avg_rate=avg_rate,
        )

    # Extract peak rates from time series
    for rate_name, pattern in RATE_PATTERNS.items():
        rates = []
        for match in re.finditer(pattern, content):
            rate = parse_number(match.group(1))
            rates.append(rate)

        # Map rate types to phases
        phase_mapping = {
            "headers": "block_headers",
            "accounts": "account_ranges",
            "storage": "storage_ranges",
            "bytecodes": "bytecodes",
        }

        if rate_name in phase_mapping:
            phase_name = phase_mapping[rate_name]
            if phase_name in metrics.phases and rates:
                metrics.phases[phase_name].peak_rate = max(rates)
                metrics.phases[phase_name].rates = rates

    # Extract total sync time
    sync_complete_match = re.search(r"Sync cycle finished successfully time_elapsed_s=(\d+)", content)
    if sync_complete_match:
        metrics.total_time_secs = int(sync_complete_match.group(1))
        metrics.success = True

    # Check for errors
    error_patterns = [
        r"ERROR.*Sync cycle failed.*error=(.+)",
        r"validation failed",
        r"BodiesNotFound",
    ]
    for pattern in error_patterns:
        for match in re.finditer(pattern, content, re.IGNORECASE):
            error_msg = match.group(0)[:200]  # Truncate long errors
            if error_msg not in metrics.errors:
                metrics.errors.append(error_msg)

    return metrics


def parse_run_directory(run_dir: Path) -> dict:
    """Parse all logs in a run directory."""
    results = {}

    # Try to get run info from summary
    summary_file = run_dir / "summary.txt"
    run_id = run_dir.name.replace("run_", "")

    # Parse each network's logs
    for log_file in run_dir.glob("ethrex-*.log"):
        network = log_file.stem.replace("ethrex-", "")
        metrics = parse_log_file(log_file, network)
        if metrics:
            metrics.run_id = run_id
            results[network] = metrics.to_dict()

    return results


def parse_all_runs(logs_dir: Path, prefix: str = "") -> dict:
    """Parse all runs in the multisync_logs directory.

    Args:
        logs_dir: Path to the logs directory
        prefix: Optional prefix to add to run IDs (e.g., "server1" -> "server1_20260201_082422")
    """
    all_results = {}

    for run_dir in sorted(logs_dir.glob("run_*")):
        if run_dir.is_dir():
            run_id = run_dir.name.replace("run_", "")
            if prefix:
                run_id = f"{prefix}_{run_id}"
            run_results = parse_run_directory(run_dir)
            if run_results:
                # Update run_id in each network's metrics
                for network in run_results:
                    run_results[network]['run_id'] = run_id
                all_results[run_id] = run_results

    return all_results


def merge_results(*result_files: Path) -> dict:
    """Merge multiple result JSON files into one.

    Args:
        result_files: Paths to JSON result files to merge

    Returns:
        Combined results dictionary
    """
    merged = {}
    for result_file in result_files:
        if not result_file.exists():
            print(f"Warning: File not found: {result_file}")
            continue
        with open(result_file) as f:
            data = json.load(f)
            # Check for duplicate run IDs
            for run_id in data:
                if run_id in merged:
                    print(f"Warning: Duplicate run ID '{run_id}' - use --prefix when parsing to avoid collisions")
            merged.update(data)
    return merged


def format_duration(secs: float) -> str:
    """Format seconds as HH:MM:SS."""
    h = int(secs // 3600)
    m = int((secs % 3600) // 60)
    s = int(secs % 60)
    if h > 0:
        return f"{h}h {m}m {s}s"
    elif m > 0:
        return f"{m}m {s}s"
    return f"{s}s"


def format_number(n: int) -> str:
    """Format number with commas."""
    return f"{n:,}"


def print_metrics_table(metrics: dict, title: str = ""):
    """Print metrics in a formatted table."""
    if title:
        print(f"\n{'=' * 70}")
        print(f" {title}")
        print(f"{'=' * 70}")

    print(f"\n  Branch: {metrics.get('branch', 'unknown')}")
    print(f"  Commit: {metrics.get('commit', 'unknown')}")
    print(f"  Network: {metrics.get('network', 'unknown')}")
    print(f"  Total Time: {format_duration(metrics.get('total_time_secs', 0))}")
    print(f"  Success: {'Yes' if metrics.get('success') else 'No'}")

    phases = metrics.get('phases', {})
    if phases:
        print(f"\n  {'Phase':<20} {'Count':>15} {'Duration':>12} {'Avg Rate':>12} {'Peak Rate':>12}")
        print(f"  {'-' * 20} {'-' * 15} {'-' * 12} {'-' * 12} {'-' * 12}")

        for phase_name in ["block_headers", "account_ranges", "account_insertion",
                          "storage_ranges", "storage_insertion", "state_healing",
                          "storage_healing", "bytecodes"]:
            if phase_name in phases:
                p = phases[phase_name]
                print(f"  {phase_name:<20} {format_number(p['count']):>15} "
                      f"{format_duration(p['duration_secs']):>12} "
                      f"{format_number(int(p['avg_rate'])):>12} "
                      f"{format_number(int(p.get('peak_rate', 0))):>12}")

    errors = metrics.get('errors', [])
    if errors:
        print(f"\n  Errors ({len(errors)}):")
        for err in errors[:5]:  # Show first 5 errors
            print(f"    - {err[:80]}...")


def compare_runs(results: dict, format_type: str = "table"):
    """Compare multiple runs and show differences."""
    if not results:
        print("No results to compare")
        return

    # Group by branch
    by_branch = {}
    for run_id, networks in results.items():
        for network, metrics in networks.items():
            branch = metrics.get('branch', 'unknown')
            key = f"{branch}:{network}"
            if key not in by_branch:
                by_branch[key] = []
            by_branch[key].append((run_id, metrics))

    if format_type == "table":
        print("\n" + "=" * 90)
        print(" SNAP SYNC BENCHMARK COMPARISON")
        print("=" * 90)

        for key, runs in sorted(by_branch.items()):
            branch, network = key.split(":")
            print(f"\n{'─' * 90}")
            print(f" Branch: {branch} | Network: {network}")
            print(f"{'─' * 90}")

            # Sort by run_id (chronological)
            runs.sort(key=lambda x: x[0])

            print(f"\n  {'Run ID':<20} {'Total Time':>12} {'Headers':>10} {'Storage':>10} {'Healing':>10} {'Success':>8}")
            print(f"  {'-' * 20} {'-' * 12} {'-' * 10} {'-' * 10} {'-' * 10} {'-' * 8}")

            for run_id, metrics in runs:
                total_time = format_duration(metrics.get('total_time_secs', 0))
                phases = metrics.get('phases', {})

                headers_time = format_duration(phases.get('block_headers', {}).get('duration_secs', 0))
                storage_time = format_duration(
                    phases.get('storage_ranges', {}).get('duration_secs', 0) +
                    phases.get('storage_insertion', {}).get('duration_secs', 0)
                )
                healing_time = format_duration(
                    phases.get('state_healing', {}).get('duration_secs', 0) +
                    phases.get('storage_healing', {}).get('duration_secs', 0)
                )
                success = "Yes" if metrics.get('success') else "No"

                print(f"  {run_id:<20} {total_time:>12} {headers_time:>10} {storage_time:>10} {healing_time:>10} {success:>8}")

            # Show statistics if multiple runs
            if len(runs) > 1:
                times = [m.get('total_time_secs', 0) for _, m in runs if m.get('success')]
                if times:
                    avg_time = sum(times) / len(times)
                    min_time = min(times)
                    max_time = max(times)
                    print(f"\n  Stats: Avg={format_duration(avg_time)}, Min={format_duration(min_time)}, Max={format_duration(max_time)}")

    elif format_type == "csv":
        print("run_id,branch,network,total_time_secs,headers_secs,storage_secs,healing_secs,success")
        for run_id, networks in results.items():
            for network, metrics in networks.items():
                phases = metrics.get('phases', {})
                row = [
                    run_id,
                    metrics.get('branch', ''),
                    network,
                    str(metrics.get('total_time_secs', 0)),
                    str(phases.get('block_headers', {}).get('duration_secs', 0)),
                    str(phases.get('storage_ranges', {}).get('duration_secs', 0) +
                        phases.get('storage_insertion', {}).get('duration_secs', 0)),
                    str(phases.get('state_healing', {}).get('duration_secs', 0) +
                        phases.get('storage_healing', {}).get('duration_secs', 0)),
                    str(metrics.get('success', False)),
                ]
                print(",".join(row))

    elif format_type == "markdown":
        print("\n## Snap Sync Benchmark Results\n")
        for key, runs in sorted(by_branch.items()):
            branch, network = key.split(":")
            print(f"\n### {branch} ({network})\n")
            print("| Run ID | Total Time | Headers | Storage | Healing | Success |")
            print("|--------|------------|---------|---------|---------|---------|")

            for run_id, metrics in sorted(runs, key=lambda x: x[0]):
                total_time = format_duration(metrics.get('total_time_secs', 0))
                phases = metrics.get('phases', {})
                headers_time = format_duration(phases.get('block_headers', {}).get('duration_secs', 0))
                storage_time = format_duration(
                    phases.get('storage_ranges', {}).get('duration_secs', 0) +
                    phases.get('storage_insertion', {}).get('duration_secs', 0)
                )
                healing_time = format_duration(
                    phases.get('state_healing', {}).get('duration_secs', 0) +
                    phases.get('storage_healing', {}).get('duration_secs', 0)
                )
                success = "✅" if metrics.get('success') else "❌"
                print(f"| {run_id} | {total_time} | {headers_time} | {storage_time} | {healing_time} | {success} |")


def diff_runs(run1_id: str, run2_id: str, results: dict, network: str = "mainnet"):
    """Show detailed diff between two runs."""
    if run1_id not in results or run2_id not in results:
        print(f"Error: Run IDs not found in results")
        return

    m1 = results[run1_id].get(network)
    m2 = results[run2_id].get(network)

    if not m1 or not m2:
        print(f"Error: Network '{network}' not found in both runs")
        return

    print(f"\n{'=' * 80}")
    print(f" COMPARISON: {run1_id} vs {run2_id} ({network})")
    print(f"{'=' * 80}")

    print(f"\n  {'Metric':<25} {'Run 1':>15} {'Run 2':>15} {'Diff':>15} {'Change':>10}")
    print(f"  {'-' * 25} {'-' * 15} {'-' * 15} {'-' * 15} {'-' * 10}")

    # Compare total time
    t1 = m1.get('total_time_secs', 0)
    t2 = m2.get('total_time_secs', 0)
    diff = t2 - t1
    pct = ((t2 - t1) / t1 * 100) if t1 > 0 else 0
    sign = "+" if diff > 0 else ""
    color = "worse" if diff > 0 else "better"
    print(f"  {'Total Time':<25} {format_duration(t1):>15} {format_duration(t2):>15} {sign}{format_duration(abs(diff)):>14} {pct:>+9.1f}%")

    # Compare phases
    phases1 = m1.get('phases', {})
    phases2 = m2.get('phases', {})

    for phase_name in ["block_headers", "account_ranges", "storage_ranges",
                       "storage_insertion", "state_healing", "storage_healing", "bytecodes"]:
        p1 = phases1.get(phase_name, {})
        p2 = phases2.get(phase_name, {})

        d1 = p1.get('duration_secs', 0)
        d2 = p2.get('duration_secs', 0)

        if d1 > 0 or d2 > 0:
            diff = d2 - d1
            pct = ((d2 - d1) / d1 * 100) if d1 > 0 else 0
            sign = "+" if diff > 0 else ""
            print(f"  {phase_name:<25} {format_duration(d1):>15} {format_duration(d2):>15} {sign}{format_duration(abs(diff)):>14} {pct:>+9.1f}%")

    print(f"\n  Run 1: {m1.get('branch')} @ {m1.get('commit')}")
    print(f"  Run 2: {m2.get('branch')} @ {m2.get('commit')}")


def main():
    parser = argparse.ArgumentParser(description="Snap Sync Benchmark Tool")
    subparsers = parser.add_subparsers(dest="command", help="Commands")

    # Parse single log
    parse_parser = subparsers.add_parser("parse", help="Parse a single log file")
    parse_parser.add_argument("log_file", type=Path, help="Path to ethrex log file")
    parse_parser.add_argument("--output", "-o", type=Path, help="Output JSON file")
    parse_parser.add_argument("--network", default="unknown", help="Network name")

    # Parse all runs
    parse_all_parser = subparsers.add_parser("parse-all", help="Parse all runs in logs directory")
    parse_all_parser.add_argument("logs_dir", type=Path, nargs="?",
                                  default=Path("./multisync_logs"),
                                  help="Path to multisync_logs directory")
    parse_all_parser.add_argument("--output", "-o", type=Path,
                                  default=Path("benchmark_results.json"),
                                  help="Output JSON file")
    parse_all_parser.add_argument("--prefix", "-p", type=str, default="",
                                  help="Prefix to add to run IDs (e.g., 'server1' -> 'server1_20260201_082422')")

    # Compare runs
    compare_parser = subparsers.add_parser("compare", help="Compare benchmark results")
    compare_parser.add_argument("results_file", type=Path, help="Results JSON file")
    compare_parser.add_argument("--format", "-f", choices=["table", "csv", "markdown"],
                               default="table", help="Output format")

    # Diff two runs
    diff_parser = subparsers.add_parser("diff", help="Diff two specific runs")
    diff_parser.add_argument("run1", help="First run ID")
    diff_parser.add_argument("run2", help="Second run ID")
    diff_parser.add_argument("--results", "-r", type=Path, required=True,
                            help="Results JSON file")
    diff_parser.add_argument("--network", "-n", default="mainnet", help="Network to compare")

    # Merge results from multiple files
    merge_parser = subparsers.add_parser("merge", help="Merge results from multiple JSON files")
    merge_parser.add_argument("files", type=Path, nargs="+",
                             help="JSON result files to merge")
    merge_parser.add_argument("--output", "-o", type=Path,
                             default=Path("merged_results.json"),
                             help="Output JSON file")

    args = parser.parse_args()

    if args.command == "parse":
        metrics = parse_log_file(args.log_file, args.network)
        if metrics:
            if args.output:
                with open(args.output, 'w') as f:
                    json.dump(metrics.to_dict(), f, indent=2)
                print(f"Results saved to {args.output}")
            else:
                print_metrics_table(metrics.to_dict(), f"Run: {args.log_file.name}")

    elif args.command == "parse-all":
        results = parse_all_runs(args.logs_dir, args.prefix)
        with open(args.output, 'w') as f:
            json.dump(results, f, indent=2)
        prefix_msg = f" with prefix '{args.prefix}'" if args.prefix else ""
        print(f"Parsed {len(results)} runs{prefix_msg}, saved to {args.output}")
        compare_runs(results, "table")

    elif args.command == "compare":
        with open(args.results_file) as f:
            results = json.load(f)
        compare_runs(results, args.format)

    elif args.command == "diff":
        with open(args.results) as f:
            results = json.load(f)
        diff_runs(args.run1, args.run2, results, args.network)

    elif args.command == "merge":
        results = merge_results(*args.files)
        with open(args.output, 'w') as f:
            json.dump(results, f, indent=2)
        print(f"Merged {len(args.files)} files ({len(results)} total runs), saved to {args.output}")
        compare_runs(results, "table")

    else:
        parser.print_help()


if __name__ == "__main__":
    main()
