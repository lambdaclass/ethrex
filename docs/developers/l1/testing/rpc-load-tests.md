# RPC load tests

These tests benchmark **read-side JSON-RPC endpoints** (`eth_call`, `eth_getBlockByNumber`, `eth_getLogs`, …) under configurable request rates, and compare ethrex against other execution clients on identical hardware.

> This is distinct from the [load tests](./load-tests.md) under `tooling/load_test`, which exercise the **write path** (submitting transactions, measuring gas/s). Use those for execution throughput; use this page for RPC serving performance.

We use [`flood`](https://github.com/paradigmxyz/flood) (Paradigm) — a load-testing orchestrator over [`vegeta`](https://github.com/tsenart/vegeta). It is multi-method, multi-rate, and multi-client by design, so the same workload can be replayed against ethrex, geth, reth, and nethermind for an apples-to-apples comparison.

## Install

`flood` is an external Python tool. Its dependencies have drifted since its last release, so pin them:

```bash
pip install paradigm-flood
pip install lxml_html_clean        # flood imports a module that lxml split out into its own package
pip install 'toolstr==0.9.5'       # newer toolstr breaks flood's summary printer; 0.9.5 works

# vegeta (flood's load engine)
brew install vegeta                                   # macOS
# or:
go install github.com/tsenart/vegeta/v12@v12.8.4      # Linux
```

Prefer a dedicated virtualenv so the pins don't disturb your global Python environment.

## Running a single method

```bash
flood eth_getBlockByNumber ethrex=http://localhost:8545 --rates 10 100 1000 --duration 30 --output ./rpc-bench-out
```

- `--rates` is the list of request rates (requests/second) to sweep.
- `--duration` is the seconds spent at each rate.
- `--output` is where results land.

List the available test templates with:

```bash
flood ls
```

flood ships templates for 8 of ethrex's top-10 methods by mainnet traffic: `eth_call`, `eth_getBlockByNumber`, `eth_getBalance`, `eth_getCode`, `eth_getLogs` (with S/M/L range variants), `eth_getTransactionByHash`, `eth_getTransactionCount`, `eth_getTransactionReceipt`. `eth_chainId` and `eth_blockNumber` are cached/in-memory and can be added as custom templates or skipped.

## Comparing multiple clients

Pass several `name=url` nodes; flood runs the same workload against each and produces a comparison:

```bash
flood eth_call ethrex=http://localhost:8545 geth=http://localhost:8546 reth=http://localhost:8547 --rates 10 100 1000 --equality
```

For a fair comparison, benchmark each client **one at a time on the same machine, synced to the same chain tip** — this removes hardware variance.

## Output

Each run writes to the `--output` directory:

- `results.json` — raw per-rate throughput, latency percentiles (p50/p90/p99), and status-code counts. This is the source of truth; it is written even if the terminal summary fails to render.
- `figures/` — `throughput.png`, `latencies.png`, `success_rate.png`.
- `test.json` — the test spec, so a run can be replayed against new nodes with `flood ./rpc-bench-out`.

## Realistic workloads

State-dependent methods (`eth_call`, `eth_getBalance`, `eth_getLogs`) need realistic inputs. flood samples real block ranges and addresses via [`checkthechain`](https://github.com/checkthechain/checkthechain) (`ctc`); configure `ctc` against an archive RPC before running these, otherwise flood has no chain data to sample from. A node synced only to genesis (e.g. a fresh `--dev` node) cannot exercise these endpoints meaningfully.
