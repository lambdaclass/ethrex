#!/usr/bin/env python3
"""Summarize one L2 TPS run directory produced by run.sh.

Reads l2.log (per-block and per-batch TPS lines written by the sequencer),
prover.log (`proving_time_ms` lines), metrics.txt (batch_tx_count / batch_size /
batch_gas_used gauges) and l1-progress.csv (lastCommittedBatch / lastVerifiedBatch
samples) and prints a markdown summary. Usage: collect.py <run_dir> [<run_dir> ...]
"""
import csv
import re
import statistics
import sys
from datetime import datetime
from pathlib import Path

BLOCK_RE = re.compile(r"^(\S+)\s+INFO.*Stored new block [0-9a-f]+, transaction_count (\d+)(?:, tps ([0-9.]+))?")
BATCH_RE = re.compile(r"^(\S+)\s+INFO.*Batch (\d+) holds blocks (\d+)\.\.=(\d+): (\d+) transactions(?: in (\d+) s, tps ([0-9.]+))?")
PROVED_RE = re.compile(r"^(\S+)\s+INFO.*Proved payload #(\d+) .*proving_time_ms=(\d+)")
METRIC_RE = re.compile(r'^(batch_[a-z_]+)\{[^}]*batch_number="(\d+)"[^}]*\}\s+([0-9.e+]+)')


def parse_ts(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00"))


def summarize(run_dir: Path):
    name = run_dir.name
    blocks, batches, proved, metrics = [], {}, {}, {}
    for line in (run_dir / "l2.log").read_text(errors="replace").splitlines():
        m = BLOCK_RE.match(line)
        if m:
            blocks.append((parse_ts(m.group(1)), int(m.group(2)), float(m.group(3)) if m.group(3) else None))
            continue
        m = BATCH_RE.match(line)
        if m:
            batches[int(m.group(2))] = dict(first=int(m.group(3)), last=int(m.group(4)), txs=int(m.group(5)),
                                           span=int(m.group(6)) if m.group(6) else None,
                                           tps=float(m.group(7)) if m.group(7) else None)
    for line in (run_dir / "prover.log").read_text(errors="replace").splitlines():
        m = PROVED_RE.match(line)
        if m:
            proved[int(m.group(2))] = (parse_ts(m.group(1)), int(m.group(3)))
    mpath = run_dir / "metrics.txt"
    if mpath.exists():
        for line in mpath.read_text(errors="replace").splitlines():
            m = METRIC_RE.match(line)
            if m:
                metrics.setdefault(int(m.group(2)), {})[m.group(1)] = float(m.group(3))

    out = [f"## {name}", ""]
    # Sequencer: steady-state blocks = blocks with transactions, excluding the first and last two.
    busy = [b for b in blocks if b[1] > 0 and b[2] is not None]
    if len(busy) > 4:
        steady = busy[2:-2]
        tps_values = [b[2] for b in steady]
        tx_total = sum(b[1] for b in steady)
        span = (steady[-1][0] - steady[0][0]).total_seconds()
        # The window opens at the first block, so its transactions are not inside it.
        tx_in_window = tx_total - steady[0][1]
        out.append("### Sequencer")
        out.append("")
        out.append("| Metric | Value |")
        out.append("|---|---|")
        out.append(f"| Blocks with transactions (steady window) | {len(steady)} |")
        out.append(f"| Transactions per block, mean | {tx_total / len(steady):.0f} |")
        out.append(f"| Per-block TPS, mean / median / max | {statistics.mean(tps_values):.1f} / {statistics.median(tps_values):.1f} / {max(tps_values):.1f} |")
        if span > 0:
            out.append(f"| Aggregate TPS over the window ({span:.0f} s) | {tx_in_window / span:.1f} |")
        out.append("")
    if batches:
        out.append("### Batches sealed by the committer")
        out.append("")
        out.append("| Batch | Blocks | Transactions | Span (s) | Batch TPS |")
        out.append("|---|---|---|---|---|")
        for n in sorted(batches):
            b = batches[n]
            out.append(f"| {n} | {b['first']}..{b['last']} | {b['txs']} | {b['span'] if b['span'] is not None else '-'} | {b['tps'] if b['tps'] is not None else '-'} |")
        out.append("")
    # Blocks: gas and transactions per block from the RPC dump.
    bpath = run_dir / "blocks.csv"
    if bpath.exists():
        brows = [r for r in csv.DictReader(bpath.open()) if int(r["tx_count"]) > 0]
        if len(brows) > 4:
            steady_blocks = brows[2:-2]
            gas = [int(r["gas_used"]) for r in steady_blocks]
            txs = [int(r["tx_count"]) for r in steady_blocks]
            out.append("### Blocks (RPC dump, steady window)")
            out.append("")
            out.append("| Metric | Value |")
            out.append("|---|---|")
            out.append(f"| Blocks with transactions | {len(steady_blocks)} |")
            out.append(f"| Gas per block, mean / max | {statistics.mean(gas):,.0f} / {max(gas):,} |")
            out.append(f"| Transactions per block, mean | {statistics.mean(txs):.0f} |")
            out.append(f"| Gas per transaction, mean | {sum(gas) / sum(txs):,.0f} |")
            out.append(f"| Block gas limit | {int(steady_blocks[0]['gas_limit']):,} |")
            out.append("")
    # Prover
    if proved:
        out.append("### Prover")
        out.append("")
        out.append("| Batch | Blocks | Transactions | Gas | Proving time (s) | Proven TPS | s per Mgas | L1 commit gas | L1 verify gas |")
        out.append("|---|---|---|---|---|---|---|---|---|")
        tx_sum = ms_sum = gas_sum = 0
        rows = 0
        for n in sorted(proved):
            _, ms = proved[n]
            m = metrics.get(n, {})
            txs = m.get("batch_tx_count")
            if txs is None and n in batches:
                txs = batches[n]["txs"]
            gas = m.get("batch_gas_used")
            blocks_n = m.get("batch_size")
            if blocks_n is None and n in batches:
                blocks_n = batches[n]["last"] - batches[n]["first"] + 1
            ptps = f"{txs / (ms / 1000):.1f}" if txs else "-"
            spm = f"{(ms / 1000) / (gas / 1e6):.2f}" if gas else "-"
            cgas = m.get("batch_commitment_gas")
            vgas = m.get("batch_verification_gas")
            out.append(f"| {n} | {int(blocks_n) if blocks_n is not None else '-'} | {int(txs) if txs is not None else '-'} | {int(gas):,} | {ms / 1000:.1f} | {ptps} | {spm} | {int(cgas):,} | {int(vgas):,} |".replace("| 0 |", "| - |") if gas is not None else f"| {n} | {int(blocks_n) if blocks_n is not None else '-'} | {int(txs) if txs is not None else '-'} | - | {ms / 1000:.1f} | {ptps} | - | - | - |")
            if txs:
                tx_sum += txs
                ms_sum += ms
                gas_sum += gas or 0
                rows += 1
        out.append("")
        if rows:
            out.append("| Metric | Value |")
            out.append("|---|---|")
            out.append(f"| Batches proved with transactions | {rows} |")
            out.append(f"| Mean proving time | {ms_sum / rows / 1000:.1f} s |")
            out.append(f"| Proven TPS, aggregate (sum txs / sum proving time) | {tx_sum / (ms_sum / 1000):.1f} |")
            if gas_sum:
                out.append(f"| Proving seconds per Mgas, aggregate | {(ms_sum / 1000) / (gas_sum / 1e6):.2f} |")
                out.append(f"| Proven gas per second, aggregate | {gas_sum / (ms_sum / 1000):,.0f} |")
            out.append("")
    # GPU and host memory samples
    gpath = run_dir / "gpu.csv"
    if gpath.exists():
        grows = list(csv.DictReader(gpath.open()))
        if grows:
            def col(name):
                vals = []
                for r in grows:
                    try:
                        vals.append(float(r[name]))
                    except (ValueError, KeyError):
                        pass
                return vals
            mem, util, power, host = col("gpu_memory_used_mib"), col("gpu_utilization_pct"), col("gpu_power_w"), col("host_mem_used_gib")
            out.append("### Prover resources (30 s samples)")
            out.append("")
            out.append("| Metric | Value |")
            out.append("|---|---|")
            if mem:
                out.append(f"| GPU memory used, max | {max(mem):,.0f} MiB |")
            if util:
                out.append(f"| GPU utilization, mean / max | {statistics.mean(util):.0f}% / {max(util):.0f}% |")
            if power:
                out.append(f"| GPU power, mean / max | {statistics.mean(power):.0f} W / {max(power):.0f} W |")
            if host:
                out.append(f"| Host memory used, max | {max(host):.0f} GiB |")
            out.append("")
    # Settled TPS: transactions in the batches verified on L1 between the first and
    # the last verification of the run, over the wall time between them (L1 block
    # timestamps of the verification transactions).
    vpath = run_dir / "verifications.csv"
    if vpath.exists():
        rows = sorted((int(r["l1_timestamp"]), int(r["first_batch"]), int(r["last_batch"])) for r in csv.DictReader(vpath.open()))
        if len(rows) >= 2:
            def batch_txs(n):
                return metrics.get(n, {}).get("batch_tx_count", batches.get(n, {}).get("txs", 0))
            t0 = rows[0][0]
            t1 = rows[-1][0]
            settled = sum(batch_txs(n) for (_, fb, lb) in rows[1:] for n in range(fb, lb + 1))
            total = sum(batch_txs(n) for (_, fb, lb) in rows for n in range(fb, lb + 1))
            out.append("### Settled on L1 (headline)")
            out.append("")
            out.append("| Metric | Value |")
            out.append("|---|---|")
            out.append(f"| Verification transactions on L1 | {len(rows)} |")
            out.append(f"| Batches verified in total | {rows[-1][2] - rows[0][1] + 1} |")
            out.append(f"| Transactions settled in total | {int(total)} |")
            out.append(f"| Window: first to last verification | {t1 - t0} s |")
            if t1 > t0:
                out.append(f"| Settled TPS (transactions verified inside the window / window) | {settled / (t1 - t0):.1f} |")
            out.append("")
    # End to end from the L1 samples
    ppath = run_dir / "l1-progress.csv"
    if ppath.exists():
        rows = list(csv.DictReader(ppath.open()))
        verified = [(parse_ts(r["utc"]), int(r["verified"])) for r in rows if int(r["verified"]) > 0]
        if len(verified) >= 2:
            first_t, first_v = verified[0]
            last_t, last_v = verified[-1]
            txs_between = sum(metrics.get(n, {}).get("batch_tx_count", batches.get(n, {}).get("txs", 0)) for n in range(first_v + 1, last_v + 1))
            wall = (last_t - first_t).total_seconds()
            out.append("### End to end (verified on L1)")
            out.append("")
            out.append("| Metric | Value |")
            out.append("|---|---|")
            out.append(f"| Batches verified between the first and last sample | {last_v - first_v} |")
            out.append(f"| Transactions in them | {int(txs_between)} |")
            if wall > 0:
                out.append(f"| Verified TPS over {wall:.0f} s | {txs_between / wall:.1f} |")
            out.append("")
    return "\n".join(out)


if __name__ == "__main__":
    for d in sys.argv[1:]:
        print(summarize(Path(d)))
