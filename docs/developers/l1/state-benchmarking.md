# State benchmarking (`state-bench`)

`state-bench` (`tooling/state_bench`) measures ethrex's **state read and write**
performance on the BAL-driven parallel block-import path, repeatably and A/B
(ethrex-vs-ethrex) on a single machine.

Measuring this on a live node is noisy: hardware, peer count, block content, and
system load all vary. `state-bench` fixes the workload and the state and re-runs
it under a controlled cache regime so two builds can be compared directly.

The tool is organized around a fixed pipeline — build a synthetic state, build a
workload against it, run it under a chosen cache regime, compare runs — that is
independent of *how* the cache is set up. Today it ships one regime, **cold**
(described below); the same machinery is intended to grow to other regimes
(warm, mixed) by changing the cache setup, not the pipeline.

## Workflow

Four steps. All take the **same Amsterdam-activated genesis**
(`fixtures/genesis/l1-bal.json`) — it must activate the fork so blocks carry a
Block Access List (BAL) and the system contracts exist.

```bash
cd tooling
BIN=./target/release/state-bench
cargo build --release -p state_bench

# 1. Synthetic state fixture: N small storage accounts + one mega account with
#    GBs of storage + a shared accessor contract + a funded signer.
$BIN gen-state --datadir /data/sb/db \
  --num-small-accounts 100000 --slots-per-account 16 --mega-account-gb 8 \
  --seed 1 --genesis ../fixtures/genesis/l1-bal.json

# 2. Workload: real blocks (via the payload builder) whose txs touch chosen
#    slots, plus their captured BALs. Reads hit seeded slots; writes hit fresh
#    slots. Runs on a throwaway copy — the fixture is left untouched.
$BIN gen-workload --datadir /data/sb/db \
  --out-chain /data/sb/chain.rlp --out-bals /data/sb/bals.rlp \
  --num-blocks 2000 --reads-per-block 200 --writes-per-block 50 \
  --mega-fraction 0.5 --genesis ../fixtures/genesis/l1-bal.json

# 3. Timed import (repeat per branch).
$BIN run --datadir /data/sb/db --chain /data/sb/chain.rlp --bals /data/sb/bals.rlp \
  --genesis ../fixtures/genesis/l1-bal.json --runs 5 --out-log /data/sb/branch.log

# 4. Compare two run logs (branch A vs branch B).
$BIN compare /data/sb/a.log /data/sb/b.log
```

`tooling/state_bench/Makefile` wraps this with env-var parameters and an A/B
recipe; `tooling/state_bench/README.md` documents every flag.

## What each run measures

Each `run` imports the workload through the normal block-import path, driven by
the parallel BAL path (`add_block_pipeline(block, Some(bal))`, i.e. `ethrex
import --with-bal`). It emits one line per run:

```
run=<i> jobs=<J> total_seconds=<f> loop_seconds=<f> commit_seconds=<f> ggas=<f> \
  block_cache_miss=<n> block_cache_hit=<n> bytes_read=<n> sst_read_count=<n>
```

- `loop_seconds` is the dominant number: the per-block import loop, where reads
  happen and where most writes land too — the store commits trie/flat-KV layers
  to disk on a rolling basis once the in-memory layer chain is `commit_threshold`
  (128) deep, which happens continuously during import.
- `commit_seconds` is the final forkchoice + drain; normally tiny.
- The most recent ~128 blocks' layers stay in memory (never flushed) — an
  unmeasured tail, so prefer workloads of thousands of blocks.

`compare` diffs two run logs (mean/median/stddev/CoV, %change, significance
notes) and warns on a `jobs` mismatch.

## Cold mode

The regime `state-bench` ships today: the state being read is on disk and *not*
in the RocksDB block cache or the OS page cache, which is where storage cost
actually shows up. It is enforced by:

- Running each measured run in a **fresh subprocess** (the store's background
  threads hold the RocksDB lock, so an in-process reopen is impossible; a new
  process also starts with a cold block cache).
- Opening RocksDB with `O_DIRECT` reads (`--direct-reads`, default on) and a
  small block cache (`--block-cache-bytes`, default 64 MiB), so reads bypass the
  OS page cache and the block cache is effectively empty.
- `--drop-caches` additionally drops the OS page cache before each run (needs
  privilege to write `/proc/sys/vm/drop_caches`; warns and continues without).
- A per-run **coldness self-check** that fails loud if cache-miss / bytes-read /
  SST-read counts are near zero, catching a silently-warm run.

Size the mega account so the working set exceeds RAM for genuinely cold reads;
`gen-state` streams storage in chunks to keep generation memory bounded.

## Resetting between runs

The same blocks must replay from the same pre-state each run. Two modes:

- `--reset checkpoint` (default): hardlinks an immutable RocksDB checkpoint of
  the pristine fixture each run. Never mutates the original datadir and gives the
  most reproducible numbers. Preferred for A/B.
- `--reset undo`: replays a pre-image undo log captured during an unmeasured
  warmup. Mutates the datadir in place (a mid-run failure leaves it dirty —
  regenerate with `gen-state`) and shows run-to-run cache-miss variance from
  async compaction.

Neither needs a copy-on-write filesystem.

## Caveats

- `--jobs` sizes the parallel-merkle thread pool but is **floored at 17 threads**
  (the BAL merkleizer needs 16 concurrent workers or it deadlocks), so it scales
  up from 17 only.
- A and B must share the same DB schema version and be run with the same
  `--jobs`, fixture, and workload — `compare` warns on a `jobs` mismatch.

## Extending to other regimes

`state-bench` is named for *state*, not *cold*, on purpose: the cache regime is a
mode, not the tool's identity. The fixture, workload, reset, and `compare`
machinery are independent of how the store is exercised, so a **warm** regime
(pre-warm the block cache / page cache, skip the cold setup), mixed hot/cold
ratios, or read-only vs write-heavy profiles can be added by flipping the
existing `--direct-reads` / `--block-cache-bytes` / `--drop-caches` knobs (e.g. a
future `--warm` or `mode` flag) and relaxing the coldness self-check — without
reworking the pipeline.
