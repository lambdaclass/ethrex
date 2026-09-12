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
| leanVM revision | `7f9777da6ab3d7bb8d10c3f5c7edce2554fcfc03` |
| ethrex commit | `2b3f394ea` (branch `eip-8288`) |
| rustc | 1.93.0 (254b59607 2026-01-19) |
| Machine | Apple M3 Max, 14 cores, 36 GiB |
| OS | macOS 26.6.2 |
| Profile | `--release` |
| Recorded | 2026-09-12 |

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
| 1 | 75 ms | 5 ms | 225,136 B | 481 MB |
| 2 | 76 ms | 5 ms | 221,608 B | 491 MB |
| 4 | 80 ms | 4 ms | 234,008 B | 543 MB |
| 8 | 141 ms | 3 ms | 234,128 B | 965 MB |
| 16 | 224 ms | 4 ms | 256,920 B | 1.71 GB |
| 32 | 485 ms | 5 ms | 270,112 B | 3.21 GB |
| 48 | 458 ms | 5 ms | 287,040 B | 3.47 GB |
| 64 | 2,029 ms | 8 ms | 286,560 B | 5.49 GB |
| 128 | 3,156 ms | 10 ms | 304,872 B | 6.44 GB |

The 48-dependency row being marginally faster than the 32 one is single-sample noise,
not an inversion.

Circuit warm-up, which `LeanVmAggregator::new` pays once per process, was 316 to 337 ms
across every run and is excluded from the proving column.

## Results: absorbing a child

The cost of the round a mempool node actually runs: absorb the previous round's
aggregate as a nested proof and add one new dependency.

| dependencies the child covers | prove that set flat | absorb it and add one |
|---|---|---|
| 8 | 133 ms | 295 ms |
| 16 | 223 ms | 273 ms |
| 32 | 417 ms | 237 ms |

Absorption does not grow with what the child covers. Flat proving does. This is the
result that decides whether `AGGREGATION_INTERVAL` is meetable, and it is the reason
it is: a node absorbs rather than re-proves, so its per-round cost does not track the
size of its pool.

## What survives a re-parametrization

Three independence properties, each visible as a shape rather than a value:

| property | evidence |
|---|---|
| Proof size is independent of claim count | 128-fold increase in dependencies, 1.35-fold increase in size |
| Verification cost is independent of claim count | 3 to 10 ms across that whole range |
| Absorbing a child is independent of what it covers | 295, 273, 237 ms for children covering 8, 16, 32 |

These follow from recursion itself rather than from tuning, so they should hold for
any circuit that supports the design. They are what the spec contribution proposes
stating as requirements.

Everything else on this page is a snapshot: the absolute milliseconds, the megabytes,
the point at which flat proving crosses `AGGREGATION_INTERVAL`.

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
