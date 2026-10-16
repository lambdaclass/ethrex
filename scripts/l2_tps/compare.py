#!/usr/bin/env python3
"""Cross-run comparison table from run directories produced by run.sh.

Usage: compare.py <run_dir> [<run_dir> ...]
Prints one markdown table with the headline and per-stage figures per run, using
the same definitions as collect.py (which it imports from the same directory).
"""
import csv
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import collect  # noqa: E402


def figures(run_dir: Path):
    name = run_dir.name
    parts = name.split("-", 1)[1] if "-" in name else name
    backend, workload = parts.split("-", 1)
    blocks, batches, proved, metrics = [], {}, {}, {}
    for line in (run_dir / "l2.log").read_text(errors="replace").splitlines():
        m = collect.BLOCK_RE.match(line)
        if m:
            blocks.append((collect.parse_ts(m.group(1)), int(m.group(2)), float(m.group(3)) if m.group(3) else None))
            continue
        m = collect.BATCH_RE.match(line)
        if m:
            batches[int(m.group(2))] = dict(first=int(m.group(3)), last=int(m.group(4)), txs=int(m.group(5)))
    for line in (run_dir / "prover.log").read_text(errors="replace").splitlines():
        m = collect.PROVED_RE.match(line)
        if m:
            proved[int(m.group(2))] = int(m.group(3))
    mpath = run_dir / "metrics.txt"
    if mpath.exists():
        for line in mpath.read_text(errors="replace").splitlines():
            m = collect.METRIC_RE.match(line)
            if m:
                metrics.setdefault(int(m.group(2)), {})[m.group(1)] = float(m.group(3))

    def batch_txs(n):
        return metrics.get(n, {}).get("batch_tx_count", batches.get(n, {}).get("txs", 0))

    out = dict(backend=backend, workload=workload)
    busy = [b for b in blocks if b[1] > 0 and b[2] is not None]
    if len(busy) > 4:
        steady = busy[2:-2]
        span = (steady[-1][0] - steady[0][0]).total_seconds()
        out["seq_tps"] = sum(b[1] for b in steady) / span if span > 0 else None
        out["txs_per_block"] = statistics.mean(b[1] for b in steady)
    bpath = run_dir / "blocks.csv"
    if bpath.exists():
        rows = [r for r in csv.DictReader(bpath.open()) if int(r["tx_count"]) > 0][2:-2]
        if rows:
            out["gas_per_block"] = statistics.mean(int(r["gas_used"]) for r in rows)
            out["gas_per_tx"] = sum(int(r["gas_used"]) for r in rows) / sum(int(r["tx_count"]) for r in rows)
    tx_sum = ms_sum = gas_sum = 0
    rows = 0
    cgas, vgas, bgas = [], [], []
    for n, ms in proved.items():
        txs = batch_txs(n)
        gas = metrics.get(n, {}).get("batch_gas_used")
        if txs and n > 1:  # batch 1 holds the deposits, not load
            tx_sum += txs
            ms_sum += ms
            gas_sum += gas or 0
            rows += 1
            for key, dst in (("batch_commitment_gas", cgas), ("batch_verification_gas", vgas), ("batch_commitment_blob_gas", bgas)):
                v = metrics.get(n, {}).get(key)
                if v:
                    dst.append(v)
    if rows:
        out["batches"] = rows
        out["txs_per_batch"] = tx_sum / rows
        out["proving_s"] = ms_sum / rows / 1000
        out["proven_tps"] = tx_sum / (ms_sum / 1000)
        if gas_sum:
            out["s_per_mgas"] = (ms_sum / 1000) / (gas_sum / 1e6)
        if cgas:
            out["l1_commit_gas"] = statistics.mean(cgas)
        if vgas:
            out["l1_verify_gas"] = statistics.mean(vgas)
        if bgas:
            out["l1_blob_gas"] = statistics.mean(bgas)
    vpath = run_dir / "verifications.csv"
    if vpath.exists():
        vrows = sorted((int(r["l1_timestamp"]), int(r["first_batch"]), int(r["last_batch"])) for r in csv.DictReader(vpath.open()))
        if len(vrows) >= 2:
            t0, t1 = vrows[0][0], vrows[-1][0]
            settled = sum(batch_txs(n) for (_, fb, lb) in vrows[1:] for n in range(fb, lb + 1))
            if t1 > t0:
                out["settled_tps"] = settled / (t1 - t0)
                out["window_s"] = t1 - t0
    gpath = run_dir / "gpu.csv"
    if gpath.exists():
        grows = list(csv.DictReader(gpath.open()))
        try:
            out["gpu_mem_max"] = max(float(r["gpu_memory_used_mib"]) for r in grows)
            out["gpu_util_mean"] = statistics.mean(float(r["gpu_utilization_pct"]) for r in grows)
            out["gpu_power_mean"] = statistics.mean(float(r["gpu_power_w"]) for r in grows)
        except (ValueError, KeyError):
            pass
    return out


ROWS = [
    ("Settled TPS on L1 (headline)", "settled_tps", "{:.1f}"),
    ("Settlement window (s)", "window_s", "{:.0f}"),
    ("Sequencer TPS (steady window)", "seq_tps", "{:.1f}"),
    ("Transactions per block", "txs_per_block", "{:.0f}"),
    ("Gas per block", "gas_per_block", "{:,.0f}"),
    ("Gas per transaction", "gas_per_tx", "{:,.0f}"),
    ("Batches proved (excluding the deposit batch)", "batches", "{:d}"),
    ("Transactions per batch", "txs_per_batch", "{:.0f}"),
    ("Proving time per batch (s)", "proving_s", "{:.1f}"),
    ("Proven TPS (txs / proving time)", "proven_tps", "{:.1f}"),
    ("Proving seconds per Mgas", "s_per_mgas", "{:.2f}"),
    ("L1 commit gas per batch", "l1_commit_gas", "{:,.0f}"),
    ("L1 blob gas per batch", "l1_blob_gas", "{:,.0f}"),
    ("L1 verify gas per batch", "l1_verify_gas", "{:,.0f}"),
    ("GPU memory, max (MiB)", "gpu_mem_max", "{:,.0f}"),
    ("GPU utilization, mean (%)", "gpu_util_mean", "{:.0f}"),
    ("GPU power, mean (W)", "gpu_power_mean", "{:.0f}"),
]

if __name__ == "__main__":
    runs = [figures(Path(d)) for d in sys.argv[1:]]
    header = "| Metric | " + " | ".join(f"{r['backend']} {r['workload']}" for r in runs) + " |"
    print(header)
    print("|---|" + "---|" * len(runs))
    for label, key, fmt in ROWS:
        cells = []
        for r in runs:
            v = r.get(key)
            if v is None:
                cells.append("-")
            elif fmt == "{:d}":
                cells.append(f"{int(v)}")
            else:
                cells.append(fmt.format(v))
        print(f"| {label} | " + " | ".join(cells) + " |")
