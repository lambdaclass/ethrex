# Cold contract-code access: state and next step

Working notes for `COLD_ACCOUNT_CODE_ACCESS` / `COLD_ACCOUNT_CODE_WRITE` in the
[EIP-8038 repricing fit](https://misilva73.github.io/eip-8038-repricing/).

## The red

`COLD_ACCOUNT_CODE_ACCESS` is ethrex's only red. It comes from EEST
`test_account_access` (stateful suite `3f6a0898955dff4f`), worst case
`AccountMode.EXISTING_CONTRACT_DIFF_MAX`, where every target address holds a unique
24576-byte runtime that is 24544 bytes `JUMPDEST`.

| param | ethrex | rank | worst case |
| --- | --- | --- | --- |
| `COLD_ACCOUNT_CODE_ACCESS` | 7680 | 2/6 | DELEGATECALL, `DIFF_MAX` |
| `COLD_ACCOUNT_CODE_WRITE` | 9986 | 3/6 | CALL, `DIFF_MAX` |

Gas is `runtime_s * 75e6` (the fit's `anchor_rate`), and the published figure is the
worst opcode/account-mode combination per client, not an average.

### What actually counts as green

The pass/fail colouring is the Goals page, not a ratio against current gas. Targets are
absolute and anchor-independent, live in `GOAL_SPECS` in the site's
`scripts/build_site.py`, and a client clears a goal when its estimate is `<=` the goal:

| goal | target | subtracts | ethrex | clears |
| --- | --- | --- | --- | --- |
| `COLD_ACCOUNT_ACCESS` | 3000 | — | CODE 7680, NOCODE 1377 | **no** |
| `ACCOUNT_WRITE` | 9000 | `COLD_ACCOUNT_ACCESS` (3000) | 6986, 727 | yes |
| `COLD_STORAGE_ACCESS` | 2100 | — | 974 | yes |
| `STORAGE_WRITE` | 8000 | `COLD_STORAGE_ACCESS` (2100) | 0 | yes |
| `WARM_ACCESS` | 100 | — | 8 | yes |

The account goals require *both* the CODE and NOCODE variants to clear, so
`COLD_ACCOUNT_ACCESS` is red on the CODE variant alone. The write goals subtract the
access goal's target from the fitted bundle, which is why `ACCOUNT_WRITE` was never
ethrex's problem despite `COLD_ACCOUNT_CODE_WRITE` sitting near its 9300 current cost.

**3000 gas is 40.0 us per cold access at the 75 Mgas/s anchor.** That is the number to
beat. Do not use 2600: it is the current osaka cost and `fit.yaml`'s `new_params`
baseline for the report's diff column, not a threshold.

ethrex does not set the proposed price either: the proposal takes erigon's 11682, and
the worst/second-worst ratio is 1.52x. Getting under besu's 3564 is what takes ethrex
out of second place.

The `EXISTING_CONTRACT_JUMPDEST` mode (target code is a single `JUMPDEST`) is excluded
by the site's `fit.yaml` as unrepresentative, so `DIFF_MAX` is the binding mode.

Not the cause: SWAP (compute suite, feeds no repricing parameter; SWAP appears as a
glue opcode only in 2 `test_sstore_bloated` rows). ethrex's account-only path is the
fastest of the six clients (2.53 us on `NON_EXISTING`).

## What is already merged into the branches

`glamsterdam-devnet-7` (`f4541adfc`), `glamsterdam-devnet-8` (`c38ceac1f`), and PR
[#7095](https://github.com/lambdaclass/ethrex/pull/7095) off main:

1. Jump destinations as a bitmap instead of a persisted RLP list of `u32` offsets.
   Stored value 3.98x code -> 1.13x code; load 224 us -> 0.48 us (new value) or
   17.4 us (legacy value, bitmap rebuilt from the bytecode, no migration); `JUMP`
   validity 12.3 ns -> 0.23 ns.
2. `Code::size()` counts the bytecode, so the code cache honors its budget.
3. Bloom filter and 4KB blocks for `account_codes` and `account_code_metadata`.
4. `EXTCODESIZE` reads `ACCOUNT_CODE_METADATA` instead of materializing the bytecode.
5. The JUMPDEST scan accumulates a byte's bits in a register and stores once, so the
   loop carries neither a read-modify-write nor a bounds-check panic path. This one
   only undoes a regression (1) introduced; it is worth ~3% of the red.

On main the code-cache budget is a 256 MiB constant, because `host_memory_limit_bytes`
lives in unmerged #7093. Both devnet branches carry #7093 and use the derived budget.

## Cost model for the binding fixture

Block time 10.2 s at 300M gas, 115k cold accesses, 18.7 us per-access base
(`MINIMAL`), 105 us blob read + 246 us list decode per *unique* contract. Solving for
the number of distinct contracts touched gives ~22.9k.

| state | per access | implied gas |
| --- | --- | --- |
| before | 89 us | 7680 |
| decode fix only, blob reads serial | 43 us | ~3200 (still red) |
| decode fix + blob reads overlapped | 26 us | ~1900 (green) |

So the decode fix alone may not clear 2600. The deciding factor is whether the BAL
warmer keeps ahead of the executor on code.

## The idea: batch and stream the BAL code prefetch

`LEVM::warm_block_from_bal` (`crates/vm/backends/levm/mod.rs:2824`) is spawned on its
own thread (`crates/blockchain/blockchain.rs:862`), enabled by default
(`bal_prefetch_enabled: true`, `rayon` arrives via the default `secp256k1` feature).
It already covers the right addresses: `bal.accounts()` returns every `AccountChanges`
entry and EIP-7928 BALs include read-only accessed addresses.

Two structural problems:

1. **A barrier between the phases.** Phase 1 must finish `prefetch_accounts` for every
   BAL address before Phase 2 collects code hashes, so the executor faults in the early
   codes itself. Code hashes are available as soon as each account state lands, so
   stream them instead of materializing `code_hashes` after a barrier.
2. **22.9k independent point gets.** Phase 2 does
   `code_hashes.par_iter().for_each(|&h| store.get_account_code(h))`, one `get_cf` per
   hash. There is no batched code read on the `Database` trait, while accounts already
   collapse into a single `multi_get` on `ACCOUNT_FLATKEYVALUE`. Add the equivalent for
   `ACCOUNT_CODES`, issued in sorted key order so RocksDB can overlap the blob reads.

Sketch:

- Add `get_account_codes_batch(&self, hashes: &[H256]) -> Result<Vec<Option<Code>>, _>`
  to the VM `Database` trait with a default loop, overridden in the rocksdb-backed
  store with `multi_get_cf` over `ACCOUNT_CODES` on sorted keys.
- In `warm_block_from_bal`, replace the phase barrier with chunked pipelining: for each
  chunk of BAL addresses, prefetch accounts, take their code hashes, and hand the chunk
  to the batched code read. Keep `cancelled` checks between chunks.
- Watch the `Store::account_code_cache` mutex: it is `Arc<Mutex<LruCache>>`, so even a
  read takes the exclusive lock and eviction runs inside it. With 6 rayon threads
  inserting, this is the one lock on the path. Sharding it (or making the batch insert
  once per chunk) belongs with this change.

Cheaper adjacent win, same file: `DatabaseLogger` already records code hashes, so a
batched path needs the same recording to keep witnesses complete.

## Result of the first four commits

Stateful run `1785777240_7cd53c4f` (2026-08-03T17:14Z) is the first with (1)-(4).
Refitting it against run `34d89ab1` (06:25Z, none of them) through the site's own
pipeline gives:

| param | before | after |
| --- | --- | --- |
| `COLD_ACCOUNT_CODE_ACCESS` | 7736 (103.1 us) | 4652 (62.0 us) |
| `COLD_ACCOUNT_CODE_WRITE` | 10415 (138.9 us) | 6355 (84.7 us) |
| `COLD_ACCOUNT_NOCODE_ACCESS` | 1377 (18.4 us) | 1389 (18.5 us) |

Binding model n=11, R² 0.997, confidence intervals disjoint (98.8-105.7 us against
59.0-63.8 us). The pre-fix refit reproduces the published 7680 to within 0.7%, so these
are comparable to the site.

Still red: 4652 against the 3000 goal, down from 2.58x to 1.55x. 22 us per access left
to find against the 40.0 us the goal allows. A natively-written snapshot removes the
17.4 us legacy rebuild (~3.5 us amortized over the 22.9k unique contracts in 115k
accesses) and hiding the 105 us blob read behind the warmer is worth ~21 us, so the
prefetch below is roughly the whole remaining gap.

`WARM_ACCESS` moving 8 -> 9 is noise: overlapping intervals, the selected opcode flips
CALLCODE -> CALL, and 7.47 -> 8.29 straddles a rounding boundary. It is 11x under its
goal either way.

Raw 300M `DIFF_MAX` wall times confirm it is the code path and not machine variance:
every code-loading opcode drops ~38% (`EXTCODESIZE` 68%, since (4) drops its bytecode
read) while `BALANCE` and `EXTCODEHASH`, which never materialize the bytecode, sit at
1936/1940/1942 and 1958/1952/1947 ms across the three runs.

Measured 62.0 us against the model's 43 us prediction for "decode fix, blob reads
serial". The pinned snapshot holds legacy RLP-list values, so every miss pays the
17.4 us rebuild instead of 0.48 us; that plus the 105 us blob read is the remaining
cost, and the prefetch below is the lever for the blob read.

Not yet measured: `f4541adfc` (JUMPDEST scan panic path). It landed 2026-08-03T21:52Z
and the image was pushed at 22:51Z, but the stateful suite has not re-run since 17:14Z.
The compute suite has `test_jumpdests` (JUMP execution) but nothing that isolates
analysis of jumpdest-free code, so only the stateful suite will show it.

## How to measure

1. Build an image from a branch containing all five commits. The 2026-08-03T15:40:30Z
   image (`commit=986b7f887`) has only the first four, so it carries the sparse
   jumpdest-analysis regression.
2. Wait for the stateful suite `3f6a0898955dff4f` to rerun. It last ran
   2026-08-03T06:25:59Z; cadence is irregular, do not assume 10.8 h.
3. `warmer_duration` (returned by `execute_block_pipeline`,
   `crates/blockchain/blockchain.rs:854`, logged at :2832 with `warmer_early_ms`) only
   rules out the warmer not running at all. On the local `bal-devnet-7-mainnet-mix-460`
   fixture it is healthy: median 2.04 ms against 7.01 ms exec, finishing before exec on
   457 of 459 blocks, though p90 of the ratio is already 0.90 and the max 1.001.

   It does **not** answer whether the prefetch is early enough, because the question is
   whether contract C is cached before the executor reaches C, not whether the warmer
   finishes first overall. The barrier means Phase 1 covers every BAL address before any
   code fetch is issued, so the executor faults in code itself for the whole of Phase 1
   no matter how early the warmer finishes in aggregate. Bound that instead by measuring
   Phase 1 alone against the block's execution time.

   benchmarkoor's own parser drops the warmer line (`pkg/blocklog/ethrex.go` maps only
   `exec|merkle|store`, and the line uses a `` `- `` prefix its regex misses), but the
   raw client log is on S3 per run and can be parsed directly:

   ```
   ./bmk-api.sh url repricings/results/runs/<run_id>/container.log
   ```

   The uploader walks the whole result dir unfiltered, so `container.log` is always
   there. 14 MB for a stateful run, one `[METRIC] BLOCK` record per block with
   `validate/exec/merkle/store/warmer`. Each test replays a fresh chain, so the
   benchmark block is always `BLOCK 3`.

**Measured on run `1785777240_7cd53c4f`: the warmer is saturated, not early.**

| block set | n | exec median | warmer/exec median | ratio >= 0.95 | finished after exec |
| --- | --- | --- | --- | --- | --- |
| 0-50 Mgas | 3133 | 26.7 ms | 0.268 | 26% | 282 |
| 50-150 Mgas | 401 | 580.8 ms | 0.968 | 61% | 101 |
| 150-250 Mgas | 476 | 1101.2 ms | 0.994 | 75% | 164 |
| 250-400 Mgas | 243 | 1638.7 ms | 0.996 | 76% | 92 |
| exec >= 1000 ms | 525 | 1576.4 ms | 0.998 | 97% | 256 |

On the slowest blocks (300M `DIFF_MAX`, ~6100 ms exec) the ratio is 0.996-0.998 with only
15-24 ms of slack, and across `BLOCK 3` at >=250 Mgas the median ratio is 1.037 with the
warmer finishing 29.6 ms *after* exec on 92 of 174 blocks. The warmer consumes the entire
execution window and lands at the wire, so the executor is racing it for code rather than
being protected by it. Light blocks sit at 0.268, which matches the 0.29 measured on the
local `bal-devnet-7-mainnet-mix-460` fixture and is why that fixture showed nothing.

This confirms warmer *throughput* is the constraint. Two candidates beyond the barrier,
both in the same loop: line 2851 re-reads all ~115k account states through
`get_account_state` purely to recover the code hashes Phase 1 already fetched, and each of
the ~22.9k `get_account_code` calls takes the single global `Mutex<CodeCache>` twice
(`store.rs:1058` to read, `store.rs:1082` to insert, eviction inside the lock) while every
rayon thread contends on it.

Implementation is smaller than the sketch below suggests: `StorageReadView::multi_get`
already exists with a default impl and a RocksDB override (`crates/storage/api/mod.rs:76`,
used by `trie.rs:174`), so the batched code read reuses that seam instead of new
`multi_get_cf` plumbing. rocksdb 0.24 / librocksdb-sys 10.4.2, whose `MultiGet` batches
blob reads; the magnitude of that on blob-backed values is the one untested assumption.
4. Compare per-test numbers with `benchmarkoor/bmk-api.sh`:
   `./bmk-api.sh index --client ethrex --suite 3f6a0898955dff4f --limit 5`, then
   `./bmk-api.sh tests <run_id> test_account_access`. Token in
   `~/.config/benchmarkoor/token`. The `/index/query/*` SQL endpoints 504; `/index/` +
   `/files/` work.
5. For the fitted gas rather than raw runtimes, refit locally in
   `~/dev/benchmarkoor/gasfit` (`.venv` holds `benchmarkoor-fetch` and `evm-gasfit` at
   the versions the site reports). `fit.yaml` is the site's own config scoped to ethrex
   with plots off; `fetch-{before,after}.yaml` pin exact runs by id substring, since the
   fetch window is day-granular and would otherwise average pre- and post-fix runs
   together. One run per suite is enough (n=11 gas levels per model).

   ```
   export BENCHMARKOOR_TOKEN=$(head -n1 ~/.config/benchmarkoor/token)
   benchmarkoor-fetch run --config fetch-after.yaml --out data/after/ \
     --cache-dir ~/.cache/benchmarkoor/fetch
   evm-gasfit run --config fit.yaml --runtimes data/after/runtimes.csv \
     --opcounts data/after/opcounts.json --out out/after/
   ```

   Results land in `out/<window>/new_gas.csv`; per-model R² and confidence intervals in
   `results.csv`. Upstream configs come from `misilva73/eip-8038-repricing`
   (`data/raw/meta.json` for the fetch query, `data/runs/<id>/fit.yaml` for the fit; the
   root copies are gitignored).

Note when reading results: (3) is a write-time RocksDB option, so it does nothing for
the pinned bloatnet snapshot until that snapshot is regenerated or the CF is compacted.
(1), (2), (4), (5) all land on the existing snapshot.

## Loose ends

- `Store::code_metadata_cache` is an unbounded `FxHashMap`, ~48 B per distinct code
  hash ever touched. Should be an LRU.
- `test_swap` is 2.3x slower than reth (but faster than geth) across 176 tests. The
  handler is already minimal, so this is interpreter dispatch architecture, not a
  localized defect. Separate work.
- ethrex's own spread across SWAP1..16 is 2.4x for opcodes that must cost the same,
  which says the raw per-test MGas/s in that suite carries structure beyond real cost.
  Median run-to-run spread on the compute suite is 13.5%, p90 32%. Do not attribute
  anything under ~35% from a single pair of runs.
