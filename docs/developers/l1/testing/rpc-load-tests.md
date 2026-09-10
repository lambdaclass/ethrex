# RPC load tests

These tests benchmark **read-side JSON-RPC endpoints** (`eth_call`, `eth_getBlockByNumber`, `eth_getLogs`, …) under configurable request rates, and compare ethrex against other execution clients on identical hardware.

> This is distinct from the [load tests](./load-tests.md) under `tooling/load_test`, which exercise the **write path** (submitting transactions, measuring gas/s). Use those for execution throughput; use this page for RPC serving performance.

The harness lives in `tooling/rpc_bench` and drives [`vegeta`](https://github.com/tsenart/vegeta) (an HTTP load generator) with workloads sampled from the node under test. It has three pieces:

- `gen_workload.py` — samples a validated workload from the node and writes `vegeta` target files.
- `run_bench.sh` — runs a `vegeta` rate-sweep over those targets.
- `summarize.py` — turns the `vegeta` reports into a latency table (single node or side-by-side).

## Methodology

Getting trustworthy, comparable numbers requires care:

- **Sample real, recent data.** Pruned/snap nodes only retain bodies and receipts within a recent window. Sampling old/archive blocks produces "missing body"/"state unavailable" responses that are still HTTP 200, so a naive benchmark measures error fast-paths, not real work. `gen_workload.py` binary-searches the node's retention floor and samples within it.
- **Validate every call.** Each sampled call is issued once and kept only if it returns a real `result` (no JSON-RPC error), so the load test contains only genuinely-serviceable requests.
- **Same depth, each node at its own head.** Run the generator against each client separately; the sampling parameters (range sizes, contracts, calldata) are fixed and `--window-blocks` pins the sampling depth, so the comparison is apples-to-apples even though the exact blocks differ per head.
- **One client at a time, same hardware.** Benchmark each client by itself on the same machine — never several at once on one box, which distorts every client's numbers. This removes hardware variance while avoiding co-tenancy contention.

For meaningful results the node should be **fully synced and settled** (background compaction/state work quiesced, caches warm) before generating the workload.

> We evaluated Paradigm's [`flood`](https://github.com/paradigmxyz/flood), which also wraps `vegeta`. Its bundled workloads sample fixed archive-era block ranges, which a pruned/snap node can't serve, so we drive `vegeta` directly with freshly-sampled, validated targets instead.

## Install

Only `vegeta` and Python 3 (standard library — no extra packages) are required:

```bash
# macOS
brew install vegeta
# Linux
go install github.com/tsenart/vegeta/v12@latest
```

## Running a benchmark

Point the tools at a synced, settled node:

```bash
cd tooling/rpc_bench

# 1. Generate a validated workload (one directory of vegeta targets per method).
python3 gen_workload.py --url http://localhost:8545 --out ./targets/ethrex

#    --window-blocks N caps the sampling depth; use the same value for every
#    client so the comparison is apples-to-apples, e.g. --window-blocks 44000.

# 2. Run the rate-sweep (default rates: 10 100 500 1000 rps, 30s each).
./run_bench.sh ./targets/ethrex ./results/ethrex

# 3. Summarize: p50/p90/p99, success rate and throughput per method per rate.
python3 summarize.py ./results/ethrex
```

## Comparing multiple clients

Benchmark each client **one at a time** on the same machine, each synced to its own head, reusing the same `--window-blocks` depth. Then point `summarize.py` at all result directories for a side-by-side p50 comparison:

```bash
# Run these sequentially, not concurrently — only one client live at a time.
python3 gen_workload.py --url http://localhost:8545 --out ./targets/ethrex     --window-blocks 44000
./run_bench.sh ./targets/ethrex ./results/ethrex
# ... stop ethrex, bring up geth on the same RPC port, repeat ...
python3 gen_workload.py --url http://localhost:8545 --out ./targets/geth       --window-blocks 44000
./run_bench.sh ./targets/geth ./results/geth

python3 summarize.py ./results/ethrex ./results/geth
```

## Output

`run_bench.sh` writes one `vegeta report -type=json` per method/rate into the output directory as `<method>__<rate>.json`, each containing latency percentiles (p50/p90/p99), success rate, throughput and status-code counts. `summarize.py` reads these.

## Coverage

The generator covers the top read methods by mainnet traffic: `eth_call`, `eth_getBlockByNumber`, `eth_getBalance`, `eth_getCode`, `eth_getTransactionCount`, `eth_getTransactionByHash`, `eth_getTransactionReceipt`, and `eth_getLogs` at small/medium/large range sizes (100/1000/10000 blocks). `eth_chainId` and `eth_blockNumber` are cached/in-memory and are not included.
