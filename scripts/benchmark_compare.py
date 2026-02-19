#!/usr/bin/env python3
"""
Compare ethrex:main vs ethrex:bal-opt metrics per block via kurtosis logs.

Tails both service logs from the start (includes backlog), parses [METRIC]
BLOCK lines, and prints a side-by-side table as matching blocks appear.
Press Ctrl-C to print averages over all compared blocks.

Usage:
    python3 scripts/benchmark_compare.py
    python3 scripts/benchmark_compare.py --enclave lambdanet-benchmark
    python3 scripts/benchmark_compare.py --main-service el-1-ethrex-lighthouse --opt-service el-2-ethrex-lighthouse
"""

import argparse
import re
import subprocess
import sys
import signal
import threading
import time

# ── log line regexes ──────────────────────────────────────────────────────────

RE_METRIC   = re.compile(r'\[METRIC\] BLOCK (\d+) \| ([\d.]+) Ggas/s \| (\d+) ms \| (\d+) txs')
RE_VALIDATE = re.compile(r'\|-\s+validate:\s+(\d+) ms')
RE_EXEC     = re.compile(r'\|-\s+exec:\s+(\d+) ms')
RE_MERKLE   = re.compile(r'\|-\s+merkle:\s+(\d+) ms.*\[concurrent: (\d+) ms, drain: (\d+) ms, overlap: (\d+)%')
RE_STORE    = re.compile(r'\|-\s+store:\s+(\d+) ms')
RE_WARMER   = re.compile(r'`-\s+warmer:\s+(\d+) ms.*\[finished: (-?\d+) ms before exec\]')


def parse_logs(enclave, service, blocks, lock, ready_event):
    """Stream kurtosis logs for a service and parse METRIC blocks into `blocks`."""
    proc = subprocess.Popen(
        ["kurtosis", "service", "logs", "-a", "-f", enclave, service],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        bufsize=1,
    )

    current_block = None
    current_data  = {}

    for raw in proc.stdout:
        line = raw.strip()

        m = RE_METRIC.search(line)
        if m:
            current_block = int(m.group(1))
            current_data  = {
                "gigagas":  float(m.group(2)),
                "total_ms": int(m.group(3)),
                "txs":      int(m.group(4)),
            }
            ready_event.set()
            continue

        if current_block is None:
            continue

        if (m := RE_VALIDATE.search(line)):
            current_data["validate_ms"] = int(m.group(1))
        elif (m := RE_EXEC.search(line)):
            current_data["execution_ms"] = int(m.group(1))
        elif (m := RE_MERKLE.search(line)):
            current_data["merkle_ms"]          = int(m.group(1))
            current_data["merkle_concurrent"]  = int(m.group(2))
            current_data["merkle_drain"]       = int(m.group(3))
            current_data["merkle_overlap_pct"] = int(m.group(4))
        elif (m := RE_STORE.search(line)):
            current_data["store_ms"] = int(m.group(1))
        elif (m := RE_WARMER.search(line)):
            current_data["warmer_ms"]    = int(m.group(1))
            current_data["warmer_early"] = int(m.group(2))
            # Warmer line is always last — block record is complete
            with lock:
                if current_block not in blocks:
                    blocks[current_block] = current_data.copy()
            current_block = None
            current_data  = {}


# ── table ─────────────────────────────────────────────────────────────────────

COLUMNS = [
    # (header, width)
    ("block",    6),
    ("txs·m",    5),  ("txs·o",    5),
    ("ggas·m",   8),  ("ggas·o",   8),  ("ggas·Δ",   8),
    ("exec·m",   7),  ("exec·o",   7),  ("exec·Δ",   7),
    ("mrkl·m",   7),  ("mrkl·o",   7),  ("mrkl·Δ",   7),
    ("ovlp·m",   6),  ("ovlp·o",   6),
    ("warm·m",   7),  ("warm·o",   7),
    ("early·m",  8),  ("early·o",  8),
]


def print_header():
    print(" | ".join(n.center(w) for n, w in COLUMNS))
    print("-+-".join("-" * w for _, w in COLUMNS))


def fmt_row(block, md, od):
    gg_m = md.get("gigagas", 0);       gg_o = od.get("gigagas", 0)
    ex_m = md.get("execution_ms", 0);  ex_o = od.get("execution_ms", 0)
    mk_m = md.get("merkle_ms", 0);     mk_o = od.get("merkle_ms", 0)
    ol_m = md.get("merkle_overlap_pct", 0); ol_o = od.get("merkle_overlap_pct", 0)
    wm_m = md.get("warmer_ms", 0);     wm_o = od.get("warmer_ms", 0)
    we_m = md.get("warmer_early", 0);  we_o = od.get("warmer_early", 0)
    gg_d = (gg_o - gg_m) / gg_m * 100 if gg_m else 0

    cells = [
        str(int(block)).rjust(6),
        str(md.get("txs", 0)).rjust(5),
        str(od.get("txs", 0)).rjust(5),
        f"{gg_m:.3f}".rjust(8),
        f"{gg_o:.3f}".rjust(8),
        f"{gg_d:+.1f}%".rjust(8),
        f"{ex_m}ms".rjust(7),
        f"{ex_o}ms".rjust(7),
        f"{ex_o - ex_m:+d}ms".rjust(7),
        f"{mk_m}ms".rjust(7),
        f"{mk_o}ms".rjust(7),
        f"{mk_o - mk_m:+d}ms".rjust(7),
        f"{ol_m}%".rjust(6),
        f"{ol_o}%".rjust(6),
        f"{wm_m}ms".rjust(7),
        f"{wm_o}ms".rjust(7),
        f"{we_m:+d}ms".rjust(8),
        f"{we_o:+d}ms".rjust(8),
    ]
    return " | ".join(cells)


# ── summary ───────────────────────────────────────────────────────────────────

def summarize(rows):
    n = len(rows)
    if not rows:
        print("No blocks compared.")
        return

    print(f"\nBlocks compared: {n}\n")

    def avg(key): return sum(r[key] for r in rows) / n

    for label, mk, ok, unit in [
        ("gigagas",          "gg_m", "gg_o",  "ggas"),
        ("execution_ms",     "ex_m", "ex_o",  "ms"),
        ("merkle_ms",        "mk_m", "mk_o",  "ms"),
        ("merkle_overlap",   "ol_m", "ol_o",  "%"),
        ("warmer_ms",        "wm_m", "wm_o",  "ms"),
        ("warmer_early",     "we_m", "we_o",  "ms"),
    ]:
        am, ao = avg(mk), avg(ok)
        if unit == "ggas":
            diff = (ao - am) / am * 100 if am else 0
            print(f"  avg {label:<18}: main={am:>8.3f}  bal-opt={ao:>8.3f}  diff={diff:+.1f}%")
        elif unit == "%":
            print(f"  avg {label:<18}: main={am:>6.1f}%  bal-opt={ao:>6.1f}%  diff={ao-am:+.1f}pp")
        else:
            diff = ao - am
            print(f"  avg {label:<18}: main={am:>7.1f}ms  bal-opt={ao:>7.1f}ms  diff={diff:+.1f}ms")


# ── service discovery ─────────────────────────────────────────────────────────

def discover_services(enclave):
    result = subprocess.run(
        ["kurtosis", "enclave", "inspect", enclave],
        capture_output=True, text=True,
    )
    services = []
    for line in result.stdout.splitlines():
        m = re.search(r'\b(el-(\d+)-ethrex-lighthouse)\b', line)
        if m:
            services.append((int(m.group(2)), m.group(1)))
    return [name for _, name in sorted(services)]


# ── main ──────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description="Compare ethrex:main vs ethrex:bal-opt per block via logs",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    parser.add_argument("--enclave",      default="lambdanet-benchmark")
    parser.add_argument("--main-service", help="Override main EL service name")
    parser.add_argument("--opt-service",  help="Override bal-opt EL service name")
    args = parser.parse_args()

    if args.main_service and args.opt_service:
        main_svc, opt_svc = args.main_service, args.opt_service
        print(f"Using services: main={main_svc}  bal-opt={opt_svc}\n")
    else:
        print(f"Discovering services in enclave '{args.enclave}'...")
        svcs = discover_services(args.enclave)
        if len(svcs) < 2:
            print(f"Found {len(svcs)} ethrex service(s): {svcs}")
            print("Expected 2. Use --main-service / --opt-service to override.")
            sys.exit(1)
        main_svc, opt_svc = svcs[0], svcs[1]
        print(f"  main:    {main_svc}")
        print(f"  bal-opt: {opt_svc}")

    main_blocks: dict = {}
    opt_blocks:  dict = {}
    lock          = threading.Lock()
    compared: set = set()
    summary_rows  = []

    main_ready = threading.Event()
    opt_ready  = threading.Event()

    for svc, blocks, ready in [
        (main_svc, main_blocks, main_ready),
        (opt_svc,  opt_blocks,  opt_ready),
    ]:
        threading.Thread(
            target=parse_logs,
            args=(args.enclave, svc, blocks, lock, ready),
            daemon=True,
        ).start()

    print("Waiting for log streams...", end="", flush=True)
    main_ready.wait(timeout=30)
    opt_ready.wait(timeout=30)
    print(" ready.\n")

    print_header()

    def on_exit(sig, frame):
        print()
        summarize(summary_rows)
        sys.exit(0)

    signal.signal(signal.SIGINT,  on_exit)
    signal.signal(signal.SIGTERM, on_exit)

    while True:
        with lock:
            new = sorted((set(main_blocks) & set(opt_blocks)) - compared)

        for block in new:
            with lock:
                md = dict(main_blocks[block])
                od = dict(opt_blocks[block])
            print(fmt_row(block, md, od))
            summary_rows.append({
                "gg_m": md.get("gigagas", 0),           "gg_o": od.get("gigagas", 0),
                "ex_m": md.get("execution_ms", 0),      "ex_o": od.get("execution_ms", 0),
                "mk_m": md.get("merkle_ms", 0),         "mk_o": od.get("merkle_ms", 0),
                "ol_m": md.get("merkle_overlap_pct", 0),"ol_o": od.get("merkle_overlap_pct", 0),
                "wm_m": md.get("warmer_ms", 0),         "wm_o": od.get("warmer_ms", 0),
                "we_m": md.get("warmer_early", 0),      "we_o": od.get("warmer_early", 0),
            })
            compared.add(block)

        time.sleep(0.5)


if __name__ == "__main__":
    main()
