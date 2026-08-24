#!/usr/bin/env python3
"""Summarize vegeta reports produced by run_bench.sh into a latency table.

Usage: summarize.py <out_dir> [<out_dir> ...]

With one directory, prints p50/p90/p99/success/throughput per method per rate.
With several (e.g. one per client), prints a side-by-side p50 comparison.

Pure stdlib; no third-party dependencies.
"""
import glob
import json
import os
import sys

RATES = [10, 100, 500, 1000]
ORDER = [
    "eth_call", "eth_getBalance", "eth_getCode", "eth_getTransactionCount",
    "eth_getBlockByNumber", "eth_getTransactionByHash", "eth_getTransactionReceipt",
    "eth_getLogs_s100", "eth_getLogs_m1000", "eth_getLogs_l10000",
]


def load(base):
    data = {}
    for f in glob.glob(os.path.join(base, "*.json")):
        name = os.path.basename(f)[:-5]
        if "__" not in name:
            continue
        method, rate = name.rsplit("__", 1)
        j = json.load(open(f))
        lat = j["latencies"]
        data.setdefault(method, {})[int(rate)] = {
            "p50": lat["50th"] / 1e6, "p90": lat["90th"] / 1e6, "p99": lat["99th"] / 1e6,
            "succ": j["success"] * 100, "thru": j["throughput"],
        }
    return data


def methods_in(data):
    return [m for m in ORDER if m in data] + [m for m in data if m not in ORDER]


def single(base):
    d = load(base)
    print(f"# {base}")
    print(f"{'method':28}{'rate':>6}{'p50ms':>9}{'p90ms':>9}{'p99ms':>9}{'succ%':>7}{'thru':>7}")
    for m in methods_in(d):
        for r in RATES:
            x = d[m].get(r)
            if not x:
                continue
            print(f"{m:28}{r:>6}{x['p50']:>9.2f}{x['p90']:>9.2f}{x['p99']:>9.2f}{x['succ']:>7.0f}{x['thru']:>7.0f}")


def compare(bases):
    loaded = {os.path.basename(b.rstrip("/")): load(b) for b in bases}
    names = list(loaded)
    allm = []
    for d in loaded.values():
        for m in methods_in(d):
            if m not in allm:
                allm.append(m)
    print("p50 (ms) @ 1000 rps")
    print("method".ljust(28) + "".join(n.rjust(14) for n in names))
    for m in allm:
        row = m.ljust(28)
        for n in names:
            x = loaded[n].get(m, {}).get(1000)
            row += (f"{x['p50']:.2f}" if x else "-").rjust(14)
        print(row)


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)
    if len(sys.argv) == 2:
        single(sys.argv[1])
    else:
        compare(sys.argv[1:])
