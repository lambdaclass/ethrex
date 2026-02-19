#!/usr/bin/env python3
"""
Compare ethrex:main vs ethrex:bal-opt metrics per block.

Polls both nodes' Prometheus endpoints, records metrics when a new block
is processed, then prints a side-by-side comparison row. On Ctrl-C prints
averages over all compared blocks.

Usage:
    python3 scripts/benchmark_compare.py
    python3 scripts/benchmark_compare.py --enclave lambdanet-benchmark
    python3 scripts/benchmark_compare.py --main-port 32776 --opt-port 32785
"""

import argparse
import re
import subprocess
import sys
import signal
import time
import urllib.request

METRICS = [
    "block_number",
    "gigagas",
    "execution_ms",
    "merkle_ms",
    "store_ms",
    "validate_ms",
    "warmer_ms",
    "warmer_early_ms",
]


def discover_ports(enclave):
    result = subprocess.run(
        ["kurtosis", "enclave", "inspect", enclave],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        print(f"Error inspecting enclave '{enclave}':\n{result.stderr}", file=sys.stderr)
        sys.exit(1)

    services = {}       # service_num -> port
    current_num = None
    for line in result.stdout.splitlines():
        m = re.search(r'\bel-(\d+)-ethrex-lighthouse\b', line)
        if m:
            current_num = int(m.group(1))
        m = re.search(r'metrics: 9001/tcp -> http://127\.0\.0\.1:(\d+)', line)
        if m and current_num is not None:
            services[current_num] = int(m.group(1))
            current_num = None
    return services


def fetch_metrics(port):
    try:
        req = urllib.request.Request(
            f"http://127.0.0.1:{port}/metrics",
            headers={"Accept": "text/plain"},
        )
        with urllib.request.urlopen(req, timeout=3) as r:
            text = r.read().decode()
    except Exception:
        return None

    data = {}
    for line in text.splitlines():
        if line.startswith("#") or not line.strip():
            continue
        parts = line.split(None, 1)
        if len(parts) == 2:
            try:
                data[parts[0]] = float(parts[1])
            except ValueError:
                pass
    return data


# ── table layout ────────────────────────────────────────────────────────────

COLUMNS = [
    # (header, width, key_or_none, is_diff, is_ms)
    ("block",      6,  None,           False, False),
    ("ggas·main",  9,  "gigagas",      False, False),
    ("ggas·opt",   9,  "gigagas",      False, False),
    ("ggas·diff", 10,  "gigagas",      True,  False),
    ("exec·main",  9,  "execution_ms", False, True),
    ("exec·opt",   9,  "execution_ms", False, True),
    ("exec·diff",  9,  "execution_ms", True,  True),
    ("mrkl·main",  9,  "merkle_ms",    False, True),
    ("mrkl·opt",   9,  "merkle_ms",    False, True),
    ("mrkl·diff",  9,  "merkle_ms",    True,  True),
    ("warm·main",  9,  "warmer_ms",    False, True),
    ("warm·opt",   9,  "warmer_ms",    False, True),
    ("warm·early",10,  "warmer_early_ms", False, True),  # opt only; positive = good
]


def print_header():
    header = " | ".join(h.center(w) for h, w, *_ in COLUMNS)
    sep    = "-+-".join("-" * w for _, w, *_ in COLUMNS)
    print(header)
    print(sep)


def build_row(block, md, od):
    cells = []
    opt_seen = set()  # tracks which keys we've already emitted the opt column for
    main_seen = set()
    col_iter = iter(COLUMNS)
    for header, width, key, is_diff, is_ms in COLUMNS:
        if header == "block":
            cells.append(str(int(block)).rjust(width))
        elif is_diff:
            mv = md.get(key, 0.0)
            ov = od.get(key, 0.0)
            if is_ms:
                diff = ov - mv
                s = f"{diff:+.0f}ms"
            else:
                pct = (ov - mv) / mv * 100 if mv else 0.0
                s = f"{pct:+.1f}%"
            cells.append(s.rjust(width))
        elif "main" in header:
            v = md.get(key, 0.0)
            s = f"{v:.3f}" if not is_ms else f"{v:.0f}"
            cells.append(s.rjust(width))
        elif "opt" in header or "early" in header:
            v = od.get(key, 0.0)
            s = f"{v:.3f}" if not is_ms else f"{v:.0f}"
            cells.append(s.rjust(width))
        else:
            cells.append("".rjust(width))
    return " | ".join(cells)


# ── summary ──────────────────────────────────────────────────────────────────

def summarize(rows):
    n = len(rows)
    if n == 0:
        print("No blocks compared.")
        return

    print(f"\nBlocks compared: {n}\n")
    for label, mk, ok, is_ms in [
        ("gigagas",      "gigagas_main",   "gigagas_opt",   False),
        ("execution_ms", "exec_ms_main",   "exec_ms_opt",   True),
        ("merkle_ms",    "merkle_ms_main", "merkle_ms_opt", True),
        ("warmer_ms",    "warmer_ms_main", "warmer_ms_opt", True),
    ]:
        avg_m = sum(r[mk] for r in rows) / n
        avg_o = sum(r[ok] for r in rows) / n
        if is_ms:
            diff = avg_o - avg_m
            print(f"  avg {label:<14}: main={avg_m:>8.1f}ms  bal-opt={avg_o:>8.1f}ms  diff={diff:+.1f}ms")
        else:
            pct = (avg_o - avg_m) / avg_m * 100 if avg_m else 0.0
            print(f"  avg {label:<14}: main={avg_m:>8.3f}    bal-opt={avg_o:>8.3f}    diff={pct:+.1f}%")


# ── main ─────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description="Compare ethrex:main vs ethrex:bal-opt metrics per block",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    parser.add_argument("--enclave",   default="lambdanet-benchmark")
    parser.add_argument("--main-port", type=int, help="Metrics port for ethrex:main (skips discovery)")
    parser.add_argument("--opt-port",  type=int, help="Metrics port for ethrex:bal-opt (skips discovery)")
    parser.add_argument("--interval",  type=float, default=1.5, help="Poll interval in seconds (default 1.5)")
    args = parser.parse_args()

    if args.main_port and args.opt_port:
        main_port, opt_port = args.main_port, args.opt_port
        print(f"Using ports: main={main_port}  bal-opt={opt_port}\n")
    else:
        print(f"Discovering ethrex services in enclave '{args.enclave}'...")
        services = discover_ports(args.enclave)
        if len(services) < 2:
            print(f"Found {len(services)} ethrex service(s): {services}")
            print("Expected 2. Use --main-port / --opt-port to override.")
            sys.exit(1)
        nums = sorted(services.keys())
        main_port = services[nums[0]]   # el-5 = participant 5 = ethrex:main
        opt_port  = services[nums[1]]   # el-6 = participant 6 = ethrex:bal-opt
        print(f"  el-{nums[0]}-ethrex-lighthouse  (main)    -> :{main_port}")
        print(f"  el-{nums[1]}-ethrex-lighthouse  (bal-opt) -> :{opt_port}")
        print()

    main_blocks: dict = {}
    opt_blocks:  dict = {}
    compared:    set  = set()
    rows:        list = []

    print_header()

    def on_exit(sig, frame):
        print()
        summarize(rows)
        sys.exit(0)

    signal.signal(signal.SIGINT,  on_exit)
    signal.signal(signal.SIGTERM, on_exit)

    while True:
        for port, store in [(main_port, main_blocks), (opt_port, opt_blocks)]:
            data = fetch_metrics(port)
            if not data:
                continue
            block = data.get("block_number", 0)
            if block > 0 and block not in store:
                store[block] = {m: data.get(m, 0.0) for m in METRICS}

        for block in sorted((set(main_blocks) & set(opt_blocks)) - compared):
            md, od = main_blocks[block], opt_blocks[block]
            print(build_row(block, md, od))
            rows.append({
                "gigagas_main":   md.get("gigagas",      0.0),
                "gigagas_opt":    od.get("gigagas",      0.0),
                "exec_ms_main":   md.get("execution_ms", 0.0),
                "exec_ms_opt":    od.get("execution_ms", 0.0),
                "merkle_ms_main": md.get("merkle_ms",    0.0),
                "merkle_ms_opt":  od.get("merkle_ms",    0.0),
                "warmer_ms_main": md.get("warmer_ms",    0.0),
                "warmer_ms_opt":  od.get("warmer_ms",    0.0),
            })
            compared.add(block)

        time.sleep(args.interval)


if __name__ == "__main__":
    main()
