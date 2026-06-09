# fixture_rebloom

Repairs the header `logs_bloom` of ethrex-generated block fixtures.

## Why

Block fixtures that ethrex produced before the header bloom was populated (and
fixtures exported with `ethrex export` from a DB holding such headers) carry a
zero `logs_bloom` even for blocks that emit logs. Since
[#6766](https://github.com/lambdaclass/ethrex/pull/6766) every import path runs
`validate_receipts_root_and_logs_bloom`, so importing those fixtures fails with
`LogsBloomMismatch` — for example the "Benchmark Block execution" CI job, which
imports `fixtures/blockchain/l2-1k-erc20.rlp`.

The bloom cannot be patched in place: the header hash commits to `logs_bloom`,
so fixing a bloom changes that block's hash and breaks the next block's
`parent_hash`. The chain has to be re-derived.

## What it does

For each block, in order, against a fresh in-memory state seeded from the given
genesis:

1. Re-links `parent_hash` to the corrected predecessor hash.
2. Re-executes the block to obtain its receipts and computes the correct
   aggregate `logs_bloom`.
3. Writes the bloom into the header and recomputes the header hash.
4. Re-adds the block through the normal validating import path (so a successful
   run proves the rewritten chain is importable) and writes it to the output.

Blocks with no logs are unchanged (their bloom was already empty and correct),
so their hashes only shift once an earlier block in the chain changes.

## Usage

```bash
cargo run --release -p fixture_rebloom -- <genesis.json> <input.rlp> <output.rlp>
```

Example — regenerate the block-execution benchmark fixture:

```bash
cd tooling
cargo run --release -p fixture_rebloom -- \
  ../fixtures/genesis/perf-ci.json \
  ../fixtures/blockchain/l2-1k-erc20.rlp \
  /tmp/l2-1k-erc20.fixed.rlp
mv /tmp/l2-1k-erc20.fixed.rlp ../fixtures/blockchain/l2-1k-erc20.rlp
```

The genesis must be the same one the fixture is imported with, otherwise block 1
will not link to it.
