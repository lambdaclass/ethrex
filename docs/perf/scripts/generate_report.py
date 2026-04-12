#!/usr/bin/env python3
"""
Generate HTML benchmark report from experiment data.

Usage:
    python3 generate_report.py
    python3 generate_report.py --experiments ../experiments --output ../report

The script reads experiment data and generates a static HTML dashboard.
"""

import argparse
import json
import os
from datetime import datetime
from pathlib import Path
from typing import Any


def load_data_json(experiments_dir: Path) -> dict[str, Any] | None:
    """Load data.json if it exists."""
    data_file = experiments_dir.parent / "report" / "data.json"
    if data_file.exists():
        with open(data_file) as f:
            return json.load(f)
    return None


def load_experiments(experiments_dir: Path) -> list[dict[str, Any]]:
    """Load experiment data from directories or data.json."""
    experiments = []

    # Try to load from data.json first
    data = load_data_json(experiments_dir)
    if data and "experiments" in data:
        return data["experiments"]

    # Otherwise scan experiment directories
    if not experiments_dir.exists():
        return experiments

    for exp_dir in sorted(experiments_dir.iterdir()):
        if not exp_dir.is_dir() or exp_dir.name.startswith("."):
            continue

        experiment = {
            "id": exp_dir.name.split("-")[0] if "-" in exp_dir.name else exp_dir.name,
            "name": "-".join(exp_dir.name.split("-")[1:])
            if "-" in exp_dir.name
            else exp_dir.name,
            "status": "unknown",
            "time_ms": None,
            "improvement_percent": None,
            "notes": "",
        }

        # Try to load results.json
        results_file = exp_dir / "results.json"
        if results_file.exists():
            with open(results_file) as f:
                results = json.load(f)
                if "results" in results and results["results"]:
                    experiment["time_ms"] = results["results"][0].get("mean", 0) * 1000

        experiments.append(experiment)

    return experiments


def load_baseline(experiments_dir: Path) -> dict[str, Any]:
    """Load baseline data."""
    data = load_data_json(experiments_dir)
    if data and "baseline" in data:
        return data["baseline"]

    # Default baseline
    return {"time_ms": 0, "time_stddev": 0, "throughput_mgas_s": 0, "runs": 0}


def format_experiment_row(exp: dict[str, Any], baseline: dict[str, Any]) -> str:
    """Format a single experiment as a table row."""
    status_class = {
        "keep": "keep",
        "discard": "discard",
        "running": "running",
        "failed": "failed",
        "unknown": "",
    }.get(exp.get("status", "").lower(), "")

    time_str = f"{exp['time_ms']:.2f}ms" if exp.get("time_ms") else "---"

    improvement = exp.get("improvement_percent")
    if improvement is not None:
        imp_str = f"{improvement:+.1f}%"
    elif exp.get("time_ms") and baseline.get("time_ms"):
        improvement = (
            (exp["time_ms"] - baseline["time_ms"]) / baseline["time_ms"]
        ) * 100
        imp_str = f"{improvement:+.1f}%"
    else:
        imp_str = "---"

    status_str = exp.get("status", "unknown").upper()
    notes_str = exp.get("notes", "")

    return f"""<tr class="{status_class}">
        <td>{exp.get('id', '---')}</td>
        <td>{exp.get('name', '---')}</td>
        <td>{time_str}</td>
        <td>{imp_str}</td>
        <td>{status_str}</td>
        <td>{notes_str}</td>
    </tr>"""


def generate_report(experiments_dir: Path, output_dir: Path) -> None:
    """Generate HTML report from benchmark results."""
    experiments = load_experiments(experiments_dir)
    baseline = load_baseline(experiments_dir)
    data = load_data_json(experiments_dir)

    # Calculate summary stats
    completed = [e for e in experiments if e.get("status", "").lower() in ["keep", "discard"]]
    best_improvement = min(
        (e.get("improvement_percent", 0) or 0 for e in experiments),
        default=0,
    )

    # Session info
    session = data.get("session", {}) if data else {}
    session_date = session.get("date", datetime.now().strftime("%Y-%m-%d"))
    session_status = session.get("status", "unknown")
    commit = session.get("commit", "unknown")
    machine = session.get("machine", "unknown")

    # Learnings
    learnings = data.get("learnings", []) if data else []

    html = f"""<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Benchmark Report - {session_date}</title>
    <meta http-equiv="refresh" content="30">
    <style>
        * {{
            box-sizing: border-box;
        }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
            max-width: 1200px;
            margin: 0 auto;
            padding: 2rem;
            background: #f9fafb;
            color: #1f2937;
        }}
        h1 {{
            margin-bottom: 0.5rem;
        }}
        .meta {{
            color: #6b7280;
            margin-bottom: 2rem;
        }}
        .card {{
            background: white;
            border: 1px solid #e5e7eb;
            border-radius: 8px;
            padding: 1.5rem;
            margin-bottom: 1.5rem;
            box-shadow: 0 1px 3px rgba(0,0,0,0.1);
        }}
        .card h2 {{
            margin-top: 0;
            margin-bottom: 1rem;
            font-size: 1.25rem;
        }}
        .stats {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
            gap: 1rem;
        }}
        .stat {{
            text-align: center;
        }}
        .stat-value {{
            font-size: 1.5rem;
            font-weight: bold;
            color: #111827;
        }}
        .stat-label {{
            font-size: 0.875rem;
            color: #6b7280;
        }}
        table {{
            width: 100%;
            border-collapse: collapse;
        }}
        th, td {{
            border: 1px solid #e5e7eb;
            padding: 0.75rem;
            text-align: left;
        }}
        th {{
            background: #f9fafb;
            font-weight: 600;
        }}
        tr:hover {{
            background: #f9fafb;
        }}
        .keep {{
            background: #d1fae5;
        }}
        .keep:hover {{
            background: #a7f3d0;
        }}
        .discard {{
            background: #fee2e2;
        }}
        .discard:hover {{
            background: #fecaca;
        }}
        .running {{
            background: #fef3c7;
        }}
        .running:hover {{
            background: #fde68a;
        }}
        .failed {{
            background: #fecaca;
        }}
        .status-running {{
            color: #d97706;
            font-weight: bold;
        }}
        .status-complete {{
            color: #059669;
            font-weight: bold;
        }}
        details {{
            margin-top: 1rem;
        }}
        summary {{
            cursor: pointer;
            color: #3b82f6;
        }}
        pre {{
            background: #f3f4f6;
            padding: 1rem;
            border-radius: 4px;
            overflow-x: auto;
            font-size: 0.875rem;
        }}
        ul {{
            margin: 0;
            padding-left: 1.5rem;
        }}
        li {{
            margin-bottom: 0.5rem;
        }}
    </style>
</head>
<body>
    <h1>Benchmark Report</h1>
    <p class="meta">Last updated: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}</p>

    <div class="card">
        <h2>Overview</h2>
        <div class="stats">
            <div class="stat">
                <div class="stat-value">
                    <span class="status-{session_status}">{session_status.upper()}</span>
                </div>
                <div class="stat-label">Status</div>
            </div>
            <div class="stat">
                <div class="stat-value">{len(completed)}/{len(experiments)}</div>
                <div class="stat-label">Completed</div>
            </div>
            <div class="stat">
                <div class="stat-value">{best_improvement:+.1f}%</div>
                <div class="stat-label">Best Improvement</div>
            </div>
        </div>
    </div>

    <div class="card">
        <h2>Baseline</h2>
        <div class="stats">
            <div class="stat">
                <div class="stat-value">{baseline.get('time_ms', 0):.2f}ms</div>
                <div class="stat-label">Block Time</div>
            </div>
            <div class="stat">
                <div class="stat-value">{baseline.get('throughput_mgas_s', 0)}</div>
                <div class="stat-label">Mgas/s</div>
            </div>
            <div class="stat">
                <div class="stat-value">{baseline.get('runs', 0)}</div>
                <div class="stat-label">Runs</div>
            </div>
        </div>
        <details>
            <summary>Environment</summary>
            <pre>Machine: {machine}
Commit: {commit}
Date: {session_date}</pre>
        </details>
    </div>

    <div class="card">
        <h2>Experiments</h2>
        <table>
            <thead>
                <tr>
                    <th>#</th>
                    <th>Name</th>
                    <th>Time</th>
                    <th>Change</th>
                    <th>Status</th>
                    <th>Notes</th>
                </tr>
            </thead>
            <tbody>
                {''.join(format_experiment_row(e, baseline) for e in experiments)}
            </tbody>
        </table>
    </div>

    {"<div class='card'><h2>Key Learnings</h2><ul>" + "".join(f"<li>{l}</li>" for l in learnings) + "</ul></div>" if learnings else ""}

    <div class="card">
        <h2>Legend</h2>
        <table>
            <tr class="keep"><td>KEEP</td><td>>5% improvement, statistically significant, correct</td></tr>
            <tr class="discard"><td>DISCARD</td><td>&lt;2% improvement or regressions</td></tr>
            <tr class="running"><td>RUNNING</td><td>Experiment in progress</td></tr>
            <tr class="failed"><td>FAILED</td><td>Crash, timeout, or incorrect output</td></tr>
        </table>
    </div>
</body>
</html>"""

    output_dir.mkdir(parents=True, exist_ok=True)
    (output_dir / "index.html").write_text(html)

    # Also save data.json for future use
    data_out = {
        "session": {
            "date": session_date,
            "status": session_status,
            "commit": commit,
            "machine": machine,
        },
        "baseline": baseline,
        "experiments": experiments,
        "learnings": learnings,
    }
    (output_dir / "data.json").write_text(json.dumps(data_out, indent=2))

    print(f"Report generated: {output_dir / 'index.html'}")
    print(f"Data saved: {output_dir / 'data.json'}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate benchmark HTML report")
    parser.add_argument(
        "--experiments",
        type=Path,
        default=Path(__file__).parent.parent / "experiments",
        help="Path to experiments directory",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(__file__).parent.parent / "report",
        help="Output directory for report",
    )
    args = parser.parse_args()

    generate_report(args.experiments, args.output)


if __name__ == "__main__":
    main()
