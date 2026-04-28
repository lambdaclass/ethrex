#!/usr/bin/env python3
"""Per-block stats from import-bench logs (pipeline format).

Usage:
    bench_stats.py <log> [<log> ...]
    bench_stats.py --compare <baseline.log> <candidate.log>

Reads BLOCK lines like:
  [METRIC] BLOCK 327 | 0.294 Ggas/s | 1.22 ms | 11 txs | 0 Mgas (1%)
  |- validate:    0.01 ms  ( 1%)
  |- exec:        0.97 ms  (80%) << BOTTLENECK
  |- merkle:      0.07 ms  ( 6%)  [concurrent: ..., drain: ..., overlap: 93%, queue: 1]
  |- store:       0.17 ms  (14%)
  `- warmer:      0.63 ms         [finished: 0.34 ms before exec]
"""

import re
import statistics
import sys
from pathlib import Path

BLOCK_RE = re.compile(
    r"BLOCK \d+ \| ([0-9.]+) Ggas/s \| ([0-9.]+) ms"
)
PHASE_RE = {
    "validate": re.compile(r"validate:\s+([0-9.]+) ms"),
    "exec": re.compile(r"exec:\s+([0-9.]+) ms"),
    "merkle": re.compile(r"merkle:\s+([0-9.]+) ms"),
    "store": re.compile(r"store:\s+([0-9.]+) ms"),
    "warmer": re.compile(r"warmer:\s+([0-9.]+) ms"),
}
OVERLAP_RE = re.compile(r"overlap:\s+(\d+)%")
DRAIN_RE = re.compile(r"drain:\s+([0-9.]+) ms")
WALL_RE = re.compile(r"Import completed blocks=(\d+) seconds=([0-9.]+)")


def parse(path: Path) -> dict:
    text = path.read_text()
    ggas = [float(m.group(1)) for m in BLOCK_RE.finditer(text)]
    total = [float(m.group(2)) for m in BLOCK_RE.finditer(text)]
    phases = {k: [float(m.group(1)) for m in r.finditer(text)] for k, r in PHASE_RE.items()}
    phases["total"] = total
    phases["ggas"] = ggas
    phases["overlap"] = [int(m.group(1)) for m in OVERLAP_RE.finditer(text)]
    phases["drain"] = [float(m.group(1)) for m in DRAIN_RE.finditer(text)]
    wall = WALL_RE.search(text)
    return {
        "path": path,
        "phases": phases,
        "blocks": int(wall.group(1)) if wall else len(ggas),
        "wall_s": float(wall.group(2)) if wall else None,
    }


def summarize(xs: list[float]) -> dict:
    if not xs:
        return {"n": 0}
    s = sorted(xs)
    return {
        "n": len(xs),
        "mean": statistics.mean(xs),
        "median": statistics.median(xs),
        "p95": s[min(int(len(s) * 0.95), len(s) - 1)],
    }


def fmt(s: dict) -> str:
    if s["n"] == 0:
        return "  (no samples)"
    return f"  n={s['n']:<5} mean={s['mean']:7.3f} median={s['median']:7.3f} p95={s['p95']:7.3f}"


def report(d: dict) -> None:
    print(f"\n=== {d['path']} ({d['blocks']} blocks", end="")
    if d["wall_s"] is not None:
        print(f", wall {d['wall_s']:.1f}s)", end="")
    print(")")
    for k in ("ggas", "total", "exec", "merkle", "store", "warmer", "validate", "drain"):
        xs = d["phases"].get(k, [])
        unit = "Ggas/s" if k == "ggas" else "ms"
        print(f"{k:>9} ({unit}):{fmt(summarize(xs))}")
    if d["phases"]["overlap"]:
        ov = summarize(d["phases"]["overlap"])
        print(f"  overlap (%):{fmt(ov)}")


def compare(baseline: dict, candidate: dict) -> None:
    print(f"\n=== {candidate['path']} vs {baseline['path']} ===")
    keys = ("ggas", "total", "exec", "merkle", "store", "warmer", "drain")
    print(f"{'metric':>9}  {'baseline':>10}  {'candidate':>10}  {'delta':>8}")
    for k in keys:
        b = summarize(baseline["phases"].get(k, []))
        c = summarize(candidate["phases"].get(k, []))
        if not b.get("median") or not c.get("median"):
            continue
        delta = (c["median"] - b["median"]) / b["median"] * 100
        good = (k == "ggas" and delta > 0) or (k != "ggas" and delta < 0)
        marker = "✓" if good else ("✗" if abs(delta) > 2 else " ")
        print(
            f"{k:>9}  {b['median']:10.3f}  {c['median']:10.3f}  {delta:+7.1f}%  {marker}"
        )
    if baseline["wall_s"] and candidate["wall_s"]:
        d = (candidate["wall_s"] - baseline["wall_s"]) / baseline["wall_s"] * 100
        print(f"{'wall':>9}  {baseline['wall_s']:10.1f}s {candidate['wall_s']:10.1f}s {d:+7.1f}%")


def main() -> int:
    args = sys.argv[1:]
    if not args:
        print(__doc__)
        return 1
    if args[0] == "--compare":
        if len(args) != 3:
            print("usage: bench_stats.py --compare <baseline> <candidate>", file=sys.stderr)
            return 1
        a, b = parse(Path(args[1])), parse(Path(args[2]))
        report(a)
        report(b)
        compare(a, b)
        return 0
    for p in args:
        report(parse(Path(p)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
