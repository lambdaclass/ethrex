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


import re

FIELD_RE = re.compile(r"(\w+)=(\d+)")


def fields(line):
    return {k: int(v) for k, v in FIELD_RE.findall(line)}


def split_name(name):
    """<timestamp>-<backend>-<workload>[-<tag>]"""
    rest = name.split("-", 1)[1]
    backend, rest = rest.split("-", 1)
    for workload in ("eth-transfers", "erc20", "fibonacci", "io-heavy"):
        if rest == workload:
            return backend, workload, ""
        if rest.startswith(workload + "-"):
            return backend, workload, rest[len(workload) + 1:]
    return backend, rest, ""


def pct(values, q):
    values = sorted(values)
    return values[min(len(values) - 1, int(len(values) * q))]


def figures(run_dir: Path):
    name = run_dir.name
    backend, workload, tag = split_name(name)
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

    out = dict(backend=backend, workload=workload, tag=tag or "control")
    cycles, witness, blobdata = [], [], {}
    for line in (run_dir / "l2.log").read_text(errors="replace").splitlines():
        if "Committer cycle" in line:
            f = fields(line)
            if "total_ms" in f:
                cycles.append([f.get("l1_reads_ms", 0), f.get("prepare_ms", 0), f.get("send_ms", 0), f["total_ms"]])
        elif "Batch witness generated" in line:
            f = fields(line)
            if "witness_ms" in f:
                witness.append(f["witness_ms"])
        elif "Batch blob data" in line:
            f = fields(line)
            if "batch" in f and "published_bytes" in f:
                blobdata[f["batch"]] = [f["blocks"], f["raw_bytes"], f["published_bytes"], f["blobs"]]
    if len(cycles) > 4:
        steady = cycles[2:]
        out["cycle_ms"] = statistics.median(c[3] for c in steady)
        out["cycle_send_ms"] = statistics.median(c[2] for c in steady)
    if len(witness) > 4:
        out["witness_ms"] = statistics.median(witness[2:])
    if blobdata:
        rows = [v for n, v in blobdata.items() if n > 1]
        if rows:
            out["blocks_per_batch"] = statistics.mean(r[0] for r in rows)
            out["blobs_per_batch"] = statistics.mean(r[3] for r in rows)
            out["compression"] = sum(r[1] for r in rows) / sum(r[2] for r in rows)
    busy = [b for b in blocks if b[1] > 0 and b[2] is not None]
    if len(busy) > 4:
        steady = busy[2:-2]
        span = (steady[-1][0] - steady[0][0]).total_seconds()
        out["seq_tps"] = sum(b[1] for b in steady[1:]) / span if span > 0 else None  # transactions after the window opens
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
    block_ts = {}
    bpath2 = run_dir / "blocks.csv"
    if bpath2.exists():
        block_ts = {int(r["number"]): int(r["timestamp"]) for r in csv.DictReader(bpath2.open())}
    vpath = run_dir / "verifications.csv"
    if vpath.exists():
        vrows = sorted((int(r["l1_timestamp"]), int(r["first_batch"]), int(r["last_batch"])) for r in csv.DictReader(vpath.open()))
        if len(vrows) >= 2:
            t0, t1 = vrows[0][0], vrows[-1][0]
            settled = sum(batch_txs(n) for (_, fb, lb) in vrows[1:] for n in range(fb, lb + 1))
            if t1 > t0:
                out["settled_tps"] = settled / (t1 - t0)
                out["window_s"] = t1 - t0
        # Latency: L1 verification time minus the L2 timestamp of the batch's last block.
        lat = []
        for ts, fb, lb in vrows:
            for n in range(fb, lb + 1):
                last_block = batches.get(n, {}).get("last")
                if n > 1 and last_block in block_ts:
                    lat.append(ts - block_ts[last_block])
        if len(lat) >= 3:
            out["latency_p50"] = statistics.median(lat)
            out["latency_p90"] = pct(lat, 0.9)
            # With the prover behind, latency grows with the backlog and so with the
            # window; the shortest one is a batch that did not queue.
            out["latency_min"] = min(lat)
    cpath = run_dir / "commits.csv"
    if cpath.exists():
        crow = sorted((int(r["l1_timestamp"]), int(r["batch"]), int(r["gas_used"]), int(r["blob_gas_used"])) for r in csv.DictReader(cpath.open()))
        crow = [r for r in crow if r[1] > 1]
        if len(crow) >= 3:
            t0, t1 = crow[0][0], crow[-1][0]
            if t1 > t0:
                out["committed_tps"] = sum(batch_txs(r[1]) for r in crow[1:]) / (t1 - t0)
            txs = sum(batch_txs(r[1]) for r in crow)
            out["l1_commit_gas"] = statistics.mean(r[2] for r in crow)
            out["l1_blob_gas"] = statistics.mean(r[3] for r in crow)
            if txs:
                out["commit_gas_per_tx"] = sum(r[2] for r in crow) / txs
                out["blob_gas_per_tx"] = sum(r[3] for r in crow) / txs
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
    ("Committed TPS on L1", "committed_tps", "{:.1f}"),
    ("Sequencer TPS (steady window)", "seq_tps", "{:.1f}"),
    ("Settlement latency p50 / p90 (s)", "latency", "{}"),
    ("Settlement latency without backlog (s)", "latency_min", "{:.0f}"),
    ("Committer cycle p50 (ms)", "cycle_ms", "{:.0f}"),
    ("Witness generation p50 (ms)", "witness_ms", "{:.0f}"),
    ("Blocks per batch", "blocks_per_batch", "{:.2f}"),
    ("Blobs per batch", "blobs_per_batch", "{:.2f}"),
    ("Compression ratio (raw / published)", "compression", "{:.2f}"),
    ("L1 commit gas per transaction", "commit_gas_per_tx", "{:.0f}"),
    ("L1 blob gas per transaction", "blob_gas_per_tx", "{:.0f}"),
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
    for r in runs:
        if "latency_p50" in r:
            r["latency"] = f"{r['latency_p50']:.0f} / {r['latency_p90']:.0f}"
    header = "| Metric | " + " | ".join(f"{r['backend']} {r['workload']} {r['tag']}" for r in runs) + " |"
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
