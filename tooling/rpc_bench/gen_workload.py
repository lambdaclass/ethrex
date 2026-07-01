#!/usr/bin/env python3
"""Generate vegeta target files for benchmarking read-side `eth_*` RPC endpoints.

The workload is sampled from *recent, real* chain data on the node under test and
every call is validated to return a real result before it is written out, so the
benchmark measures genuine serving work rather than error/empty fast-paths.

Design notes:
- **Recent sampling.** Pruned/snap nodes only retain bodies/receipts within a
  recent window; sampling archive-era blocks (as off-the-shelf tools do) yields
  "missing body"/"state unavailable" errors that get counted as HTTP-200 success.
  We binary-search the earliest block with a body and sample within that window
  (optionally capped via --window-blocks so every client uses the same depth).
- **Per-node at its own head.** Run this against each client separately; the
  sampling parameters (range sizes, contracts, calldata) are fixed, so the
  comparison is apples-to-apples even though the exact blocks differ per head.
- **Validation.** Each candidate call is issued once; only calls returning a real
  `result` (no JSON-RPC error) are kept.

Output: one `<method>.json` per endpoint in --out, in vegeta's JSON target format
(one target per line). Feed these to `run_bench.sh`.

Pure stdlib; no third-party dependencies.
"""
import argparse
import base64
import concurrent.futures
import json
import os
import random
import urllib.request

# Well-known mainnet ERC-20s and selectors for eth_call (cheap, deterministic
# reads), plus the ERC-20 Transfer topic for eth_getLogs range queries.
ERC20 = {
    "USDC": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
    "DAI": "0x6b175474e89094c44da98b954eedeac495271d0f",
    "USDT": "0xdac17f958d2ee523a2206206994597c13d831ec7",
    "WETH": "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
    "LINK": "0x514910771af9ca656af840dff83e8264ecf986ca",
    "LUSD": "0x5f98805a4e8be255a32880fdec7f6728c6568ba0",
}
CALLDATA = {
    "totalSupply": "0x18160ddd",
    "decimals": "0x313ce567",
    "symbol": "0x95d89b41",
    "name": "0x06fdde03",
}
# LUSD is a low-volume token: getLogs over a block range scans the range but
# returns few logs, so it isolates per-block scan cost from response size.
LOGS_CONTRACT = ERC20["LUSD"]
TRANSFER_TOPIC = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"


def rpc(url, method, params, timeout=60, read_limit=None):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        if read_limit:
            return r.read(read_limit).decode("utf-8", "replace")
        return json.loads(r.read())


def is_ok(resp):
    return isinstance(resp, dict) and resp.get("result") is not None and "error" not in resp


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--url", default="http://localhost:8545", help="node JSON-RPC URL")
    ap.add_argument("--out", required=True, help="output directory for target files")
    ap.add_argument("--seed", type=int, default=42, help="RNG seed (deterministic sampling)")
    ap.add_argument("--harvest", type=int, default=120, help="recent blocks to harvest txs/addresses from")
    ap.add_argument("--window-blocks", type=int, default=0,
                    help="cap the recent sampling window (0 = use the full retained body window)")
    args = ap.parse_args()
    random.seed(args.seed)
    url = args.url

    head = int(rpc(url, "eth_blockNumber", [])["result"], 16)
    print("head:", head)

    # Binary-search the earliest block that still has a body (the retention floor).
    def has_body(bn):
        try:
            return rpc(url, "eth_getBlockByNumber", [hex(bn), False]).get("result") is not None
        except Exception:
            return False

    lo, hi = 0, head
    while lo < hi:
        mid = (lo + hi) // 2
        if has_body(mid):
            hi = mid
        else:
            lo = mid + 1
    floor = lo
    auto_window = head - floor
    auto_lo = floor + max(1, auto_window // 50)  # small margin off the floor
    if args.window_blocks and args.window_blocks > 0:
        lo_blk = max(auto_lo, head - args.window_blocks)
    else:
        lo_blk = auto_lo
    window = head - lo_blk
    print(f"body floor=block {floor} auto_window={auto_window}; "
          f"sampling lo_blk={lo_blk} window={window} (cap={args.window_blocks})")

    # Harvest real tx hashes and addresses from recent full blocks.
    addrs, txhashes = set(), []
    for bn in random.sample(range(head - args.harvest, head + 1), args.harvest):
        try:
            blk = rpc(url, "eth_getBlockByNumber", [hex(bn), True]).get("result")
        except Exception:
            blk = None
        if not blk:
            continue
        for tx in blk.get("transactions", []):
            txhashes.append(tx["hash"])
            if tx.get("from"):
                addrs.add(tx["from"].lower())
            if tx.get("to"):
                addrs.add(tx["to"].lower())
    print(f"harvested {len(txhashes)} txs, {len(addrs)} addrs")

    # Classify harvested addresses into contracts (for getCode/call) vs EOAs.
    def classify(x):
        try:
            code = rpc(url, "eth_getCode", [x, "latest"]).get("result", "0x")
            return x, (code not in (None, "0x") and len(code) > 2)
        except Exception:
            return x, None

    contracts, eoas = [], []
    with concurrent.futures.ThreadPoolExecutor(max_workers=20) as ex:
        for x, c in ex.map(classify, list(addrs)):
            if c is True:
                contracts.append(x)
            elif c is False:
                eoas.append(x)
    print(f"contracts:{len(contracts)} eoas:{len(eoas)}")

    def rnd_blk():
        return hex(random.randint(lo_blk, head))

    def logs_ranges(size, n):
        # Skip ranges larger than the retained window.
        if head - size <= lo_blk:
            return []
        out = []
        for _ in range(n):
            s = random.randint(lo_blk, head - size)
            out.append([{"address": LOGS_CONTRACT, "topics": [TRANSFER_TOPIC],
                         "fromBlock": hex(s), "toBlock": hex(s + size)}])
        return out

    random.shuffle(txhashes)
    pools = {
        "eth_getBlockByNumber": [[rnd_blk(), False] for _ in range(1000)],
        "eth_getTransactionByHash": [[h] for h in txhashes[:1000]],
        "eth_getTransactionReceipt": [[h] for h in txhashes[:1000]],
        "eth_getBalance": [[random.choice(contracts + eoas), "latest"] for _ in range(1000)],
        "eth_getCode": [[random.choice(contracts or (contracts + eoas)), "latest"] for _ in range(800)],
        "eth_getTransactionCount": [[random.choice(eoas or (contracts + eoas)), "latest"] for _ in range(800)],
        "eth_call": [[{"to": c, "data": d}, "latest"] for c in ERC20.values() for d in CALLDATA.values()],
        "eth_getLogs_s100": logs_ranges(100, 200),
        "eth_getLogs_m1000": logs_ranges(1000, 100),
        "eth_getLogs_l10000": logs_ranges(10000, 40),
    }
    rpc_method = {k: ("eth_getLogs" if k.startswith("eth_getLogs") else k) for k in pools}

    def validate(key, p):
        m = rpc_method[key]
        try:
            if key.startswith("eth_getLogs"):
                # getLogs responses can be huge; just check the head of the body.
                txt = rpc(url, m, p, timeout=120, read_limit=80)
                return '"result"' in txt and '"error"' not in txt
            return is_ok(rpc(url, m, p, timeout=60))
        except Exception:
            return False

    os.makedirs(args.out, exist_ok=True)
    summary = {}
    for key, plist in pools.items():
        with concurrent.futures.ThreadPoolExecutor(max_workers=15) as ex:
            res = list(ex.map(lambda p: (p, validate(key, p)), plist))
        good = [p for p, v in res if v]
        summary[key] = (len(good), len(plist))
        with open(os.path.join(args.out, key + ".json"), "w") as f:
            for p in good:
                jr = json.dumps({"jsonrpc": "2.0", "id": 1, "method": rpc_method[key], "params": p})
                target = {"method": "POST", "url": url,
                          "header": {"Content-Type": ["application/json"]},
                          "body": base64.b64encode(jr.encode()).decode()}
                f.write(json.dumps(target) + "\n")
        print(f"{key:26} valid {len(good):4}/{len(plist):4}")
    print("SUMMARY", json.dumps(summary))
    print("META", json.dumps({"head": head, "floor": floor, "lo_blk": lo_blk, "window": window,
                              "contracts": len(contracts), "eoas": len(eoas), "txs": len(txhashes)}))


if __name__ == "__main__":
    main()
