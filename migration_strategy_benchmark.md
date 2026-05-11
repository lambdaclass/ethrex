# Receipts v1→v2 migration: strategy comparison

Benchmark of five migration strategies for re-keying the RECEIPTS column
family from RLP-encoded `(BlockHash, u64)` to raw `block_hash (32B) || index (8B BE)`.

## Setup

- Hardware: Apple M-series, 24 GB RAM, APFS NVMe
- Workload: 60 M synthetic receipts, ~480 B values (mainnet-shaped LZ4 ratio)
- Pre-migration: 11.95 GB on disk (~15 GB peak before initial compaction)
- Each strategy runs against an `cp -c` (APFS clonefile) of the same loaded template
- Migration is timed as the wall clock of the chosen strategy function
- 8 hardware threads available; two-cf-parallel uses up to 8

## Results (60 M receipts ≈ 15 GB)

| Strategy           | Time  | Rate (rec/s) | Speedup vs baseline | Post-mig disk |
|--------------------|------:|-------------:|--------------------:|--------------:|
| **two-cf**         | **117.8 s** |   509 K   | **1.31×** | 12.90 GB |
| two-cf-parallel    | 134.2 s |   447 K   | 1.15×    | 12.88 GB |
| seek-resume        | 143.9 s |   417 K   | 1.08×    | 12.50 GB |
| baseline (PR)      | 154.9 s |   387 K   | 1.00×    | 12.30 GB |
| cursor-held        | 159.9 s |   375 K   | 0.97× (slower) | 13.02 GB |

CSV at `/tmp/mig-results.csv`.

## Strategies, summarized

- **baseline** — PR #6548 as merged: cursor scan dumps old keys to a temp file
  (~2.4 GB on disk); second pass reads keys back in batches, point-lookups
  values, writes new keys + per-key delete tombstones. Two read passes over
  the dataset.
- **two-cf** — single open read cursor on `receipts`, write batches to a fresh
  `receipts_v2` CF, drop the old CF when done. One read pass, no per-key deletes.
- **two-cf-parallel** — same as two-cf, but the keyspace is segmented by RLP
  list-header byte (0xe2..0xea) × H256 first byte; one worker thread per
  segment, all writing to `receipts_v2`.
- **seek-resume** — read a batch of `BATCH_SIZE` entries, drop the iterator,
  write batch (puts + deletes) on the same CF, re-open iterator with seek
  past the last processed key. No long-lived read snapshot.
- **cursor-held** — open the read cursor once, accumulate writes in a
  WriteBatch, flush every `BATCH_SIZE` entries without dropping the cursor.
  Relies on RocksDB iterator snapshot semantics.

## Observations

**Two-cf wins by 1.31×** despite being the simplest of the four. The
combination of (a) single read pass, (b) no per-key delete tombstones,
(c) `drop_cf` reclaiming the old SSTs in O(metadata) is hard to beat.

**two-cf-parallel is *slower* than two-cf**, not faster. Three factors:

1. RocksDB's WAL serializes writes — even with N threads writing to
   different keys in the same CF, they contend on the WAL mutex.
2. The single shared memtable + flush pipeline is the actual bottleneck
   for write-heavy workloads, not the read side.
3. My segmentation by RLP list header is naturally skewed: txs with idx
   < 128 land in header 0xe2, idx 128..65535 in 0xe3 — for typical block
   sizes that's a ~75/25 split into just two active segments, with the
   other 6+ threads sitting idle.

A version that segments more aggressively (e.g., 16-way splits within
0xe2 and 0xe3) might recover some of the loss, but the WAL contention
ceiling means parallelism here is an unattractive complexity/performance
trade.

**cursor-held is the slowest** — surprising on its face. The pinned
iterator snapshot keeps older sequence numbers alive across the entire
~3-minute migration, preventing rocksdb from compacting away the writes
made earlier in the same run. Memory pressure and L0 buildup grow over
time, and the per-batch cost trends upward toward the end. A measurement
artifact more than a fundamental property — but a real cost of not
dropping the iterator periodically.

**seek-resume is a modest 1.08× win** over baseline. It avoids the temp
file and the second read pass, but pays for fresh iterator + seek per
batch (the seeks are cheap; the cost is mostly the unavoidable per-key
delete tombstones).

**Baseline's temp-file approach has structural overhead** that the
re-keying problem doesn't actually require: the iterator already had the
values in hand during Phase 1, but they're discarded so the temp file
stays small (~2.4 GB), then re-fetched via 60 M point lookups in Phase 2.
That's roughly 2× the read work of any single-pass strategy.

## Caveat — these numbers are bounded by disk

This benchmark fits the entire DB in OS page cache + RocksDB block cache
(11.95 GB DB on a 24 GB machine; the 4 GB rocksdb block cache + ~10-15 GB
of OS cache absorbs everything). On a real ethrex node where receipts
share cache budget with the rest of the 500 GB DB, every read becomes a
real disk I/O, and the gap between strategies will widen — particularly:

- baseline gets *worse* (its Phase 2 point lookups all miss cache)
- two-cf gets *relatively better* (one sequential read pass; cache misses
  hit sequential prefetch)
- two-cf-parallel may finally beat two-cf (multiple I/O streams hide
  individual seek latencies)

Re-running this on a node where cache pressure matters (mainnet-1 with
its 105 M live receipts and 500 GB of competing data) would shift the
relative ordering. Worth a follow-up.

## Recommendation

For PR #6548 as it stands: **switch to the two-cf strategy**. Concrete
reasons:

- 1.31× wall-time win at 60 M (and likely larger on real cache-pressured
  nodes).
- Smaller WAL/memtable churn — no per-key delete tombstones.
- `drop_cf` reclaims the old data immediately, so the migration's peak
  disk usage during the run is closer to 1× the receipts CF size, not
  the baseline's ~2× (old SSTs + tombstones + new SSTs all coexist until
  compaction finishes).
- The actual code is simpler than the temp-file dance.

The one schema concern: ethrex's TABLES const lists `receipts`, and the
RocksDBBackend drops "obsolete" CFs on open. The migration would need to
either (a) update TABLES to list `receipts_v2` from v2 onward, or (b)
add a final post-migration step that copies `receipts_v2` back to a
freshly-recreated `receipts` CF (doubles the migration work — not great).
Option (a) is cleaner — TABLES becomes a function of schema version, or
the migration framework gets a "rename CF" primitive.

## Files

- Strategy code: `crates/storage/migrations.rs` — functions
  `migrate_1_to_2` (baseline), `migrate_1_to_2_two_cf`,
  `migrate_1_to_2_two_cf_parallel`, `migrate_1_to_2_seek_resume`,
  `migrate_1_to_2_cursor_held`.
- Test harness: `migrate_1_to_2_synthetic_load` in the same file.
  Selects strategy via `ETHREX_MIG_STRATEGY` env var.
- Reproduce a single strategy:

  ```bash
  rm -rf /tmp/mig-test
  cp -cR /tmp/mig-template /tmp/mig-test
  ETHREX_MIG_RECEIPTS=60000000 ETHREX_MIG_DIR=/tmp/mig-test \
    ETHREX_MIG_MIGRATE_ONLY=1 ETHREX_MIG_STRATEGY=two-cf \
    ETHREX_MIG_RESULTS_FILE=/tmp/mig-results.csv \
    cargo test -p ethrex-storage --features rocksdb --release \
      migrate_1_to_2_synthetic_load -- --ignored --nocapture
  ```
