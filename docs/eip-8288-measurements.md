# EIP-8288 measurements

Evidence behind the spec contributions in [`eip-8288.md`](eip-8288.md). EIP-8288 makes
several quantitative claims about aggregation and publishes gas constants without a
stated basis, so these measure what the named tooling actually does.

**Everything here expires.** The figures belong to one leanVM revision on one machine.
What does not expire is the shape of the results, which is why the spec contributions
are written as properties rather than numbers. The split is in
[What survives a re-parametrization](#what-survives-a-re-parametrization) below.

Re-run after changing the pinned leanVM revision, and replace the recorded run.

## Environment of the recorded run

| | |
|---|---|
| leanVM revision | `b7b3b742af8dda100a0263b22c36e33963cc165c`, the head of branch `nicetry` |
| ethrex commit | `b5c3a098c` (branch `eip-8288`) with the leanVM pin bumped as in the commit that records this run |
| rustc | 1.93.0 (254b59607 2026-01-19) |
| Machine | Apple M3 Max, 14 cores, 36 GiB |
| OS | macOS 26.6.2 |
| Profile | `--release` |
| Recorded | 2026-10-02 |

Peak resident set is measured per process by `/usr/bin/time -l`, one dependency count
per invocation, so the figure belongs to a single aggregate rather than to a whole
test run. Each row is a single sample, not an average, so small non-monotonic steps
are noise rather than signal.

## How to reproduce

```bash
scripts/eip8288-bench.sh                 # both benchmarks, default counts
scripts/eip8288-bench.sh aggregate 1 16  # one benchmark, chosen counts
scripts/eip8288-bench.sh absorb 8 32
```

The script prints the leanVM revision, the ethrex commit and the rustc version before
the results, so a pasted run identifies itself. It builds the `leanvm` feature and
invokes the test binary directly, because the resident-set figure has to measure the
test rather than cargo.

The benchmarks themselves are `bench_aggregation_cost` and
`bench_recursive_absorption` in `test/tests/common/leanvm_aggregator_tests.rs`, both
`#[ignore]`d so an ordinary test run does not pay for proving. To drive one directly:

```bash
EIP8288_BENCH_DEPS=16 cargo test -p ethrex-test --features leanvm --release \
  -- --ignored --nocapture --test-threads=1 bench_aggregation_cost
```

`--test-threads=1` matters: concurrent proving runs make the memory figure
meaningless and distort the timings.

## Results: aggregating from witnesses

The cost of proving a dependency set from scratch, which is what a client does for its
own dependencies before any aggregation has happened.

| dependencies | prove | verify | proof size | peak RSS |
|---|---|---|---|---|
| 1 | 150 ms | 11 ms | 264,840 B | 818 MB |
| 2 | 284 ms | 13 ms | 260,672 B | 1.26 GB |
| 4 | 318 ms | 11 ms | 276,432 B | 2.13 GB |
| 8 | 1,017 ms | 14 ms | 287,192 B | 4.02 GB |
| 16 | 2,755 ms | 34 ms | 299,592 B | 6.45 GB |
| 32 | 5,179 ms | 23 ms | 314,512 B | 7.36 GB |
| 64 | 12,829 ms | 21 ms | 341,928 B | 11.1 GB |
| 128 | 93,215 ms | 104 ms | 357,296 B | 19.0 GB |

The 128-dependency row was taken at 19 GB resident on a 36 GiB machine, and both its
proving time and its verification time step up far more than the doubling from 64
would predict. It is a single sample and should be read as a memory-pressure point,
not as the circuit's cost curve.

Circuit warm-up, which `LeanVmAggregator::new` pays once per process, was 516 to 544 ms
across every run and is excluded from the proving column.

## Results: absorbing a child

The cost of the round a mempool node actually runs: absorb the previous round's
aggregate as a nested proof and add one new dependency.

| dependencies the child covers | prove that set flat | absorb it and add one |
|---|---|---|
| 8 | 508 ms | 395 ms |
| 16 | 856 ms | 385 ms |
| 32 | 1,558 ms | 415 ms |

Absorption does not grow with what the child covers. Flat proving does. This is the
result that decides whether `AGGREGATION_INTERVAL` is meetable, and it is the reason
it is: a node absorbs rather than re-proves, so its per-round cost does not track the
size of its pool.

## What survives a re-parametrization

Three independence properties, each visible as a shape rather than a value:

| property | evidence |
|---|---|
| Proof size is independent of claim count | 128-fold increase in dependencies, 1.35-fold increase in size |
| Verification cost is independent of claim count | 11 to 34 ms up to 64 dependencies; the 104 ms at 128 is the memory-pressure sample noted above |
| Absorbing a child is independent of what it covers | 395, 385, 415 ms for children covering 8, 16, 32 |

These follow from recursion itself rather than from tuning, so they should hold for
any circuit that supports the design. They are what the spec contribution proposes
stating as requirements.

Everything else on this page is a snapshot: the absolute milliseconds, the megabytes,
the point at which flat proving crosses `AGGREGATION_INTERVAL`.

The previous recorded run, at leanVM `7f9777da` (SPHINCS over BLAKE2s, 4,924-byte
signatures, 2026-09-12, same machine), shows how far the snapshot moves with the
circuit. Flat proving crossed 1,000 ms between 32 and 64 dependencies there and
crosses it at 8 here, where the signature's Keccak-256 hashing is proved in-circuit.
Absorbing a child took 237 to 295 ms there and 385 to 415 ms here, inside
`AGGREGATION_INTERVAL` both times. Proof size grew 1.35-fold across the 128-fold range
in both runs. The three properties held in both, which is the point.

## Demonstrations that need no benchmark

Three questions are settled by construction rather than by timing. They live in
`test/tests/common/eip8288_spec_questions.rs` and run in the ordinary suite:

```bash
cargo test -p ethrex-test --test ethrex_tests eip8288_spec_questions
```

| test | what it settles |
|---|---|
| `test_case_1_shape_is_only_broadcastable_if_dependency_frames_are_transparent` | The EIP's own Test Cases 1 and 2 are mempool-eligible only under the transparent reading. Counted, the prefix is `[dep_verify, self_verify]`, which matches none of EIP-8141's four recognized shapes |
| `dependency_gas_would_dominate_the_verify_budget_if_it_counted` | Sixteen leanSPHINCS dependencies would claim near half of `MAX_VERIFY_GAS`, and one frame of leanSTARK dependencies 76 times it, for a frame that performs no simulation work |
| `omitting_the_dependency_frame_receipt_renumbers_every_later_frame` | Omitting the receipt entry shifts every later frame's index, so `FRAMEPARAM` `0x05`, `0x0A` and `0x0B` read the wrong frame or fall out of bounds |

A fourth question, whether a dependency frame may terminate an atomic batch, has
nothing to measure. It is settled by reading EIP-8141: a batch is a maximal run
`[i, j]` where frames `i` through `j-1` carry `ATOMIC_BATCH_FLAG` and frame `j` does
not, the successor check excludes only `VERIFY`, and EIP-8288 requires `flags == 0`.
So a dependency frame is a legal terminator, and a failed batch marks it skipped with
receipt status `0x2` while its dependencies stay binding on the block and its gas
stays charged.
