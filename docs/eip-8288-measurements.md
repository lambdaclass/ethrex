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

## Comparison across recorded runs

Two runs so far, on the same machine with the same rustc and the same ethrex code
apart from the leanVM pin. Run 2 is the one recorded above.

| | Run 1 | Run 2 |
|---|---|---|
| Recorded | 2026-09-12 | 2026-10-02 |
| leanVM revision | `7f9777da`, on `main` | `b7b3b742`, head of `nicetry` |
| SPHINCS profile | SPHINCS over BLAKE2s, with WOTS+C and FORS+C | NiceTry "SPHINCS- v2": Keccak-256, standard WOTS+ and FORS |
| Signature size | 4,924 bytes | 6,176 bytes |
| Produced by a wallet today | no | yes, the Daisugi testnet's |
| Circuit warm-up, once per process | 316 to 337 ms | 516 to 544 ms |

Proving and verification time, aggregating from witnesses:

| dependencies | prove, run 1 | prove, run 2 | verify, run 1 | verify, run 2 |
|---|---|---|---|---|
| 1 | 75 ms | 150 ms | 5 ms | 11 ms |
| 2 | 76 ms | 284 ms | 5 ms | 13 ms |
| 4 | 80 ms | 318 ms | 4 ms | 11 ms |
| 8 | 141 ms | 1,017 ms | 3 ms | 14 ms |
| 16 | 224 ms | 2,755 ms | 4 ms | 34 ms |
| 32 | 485 ms | 5,179 ms | 5 ms | 23 ms |
| 64 | 2,029 ms | 12,829 ms | 8 ms | 21 ms |
| 128 | 3,156 ms | 93,215 ms | 10 ms | 104 ms |

Proof size and peak resident memory for the same runs:

| dependencies | proof size, run 1 | proof size, run 2 | peak RSS, run 1 | peak RSS, run 2 |
|---|---|---|---|---|
| 1 | 225 KB | 265 KB | 0.48 GB | 0.82 GB |
| 2 | 222 KB | 261 KB | 0.49 GB | 1.26 GB |
| 4 | 234 KB | 276 KB | 0.54 GB | 2.13 GB |
| 8 | 234 KB | 287 KB | 0.97 GB | 4.02 GB |
| 16 | 257 KB | 300 KB | 1.71 GB | 6.45 GB |
| 32 | 270 KB | 315 KB | 3.21 GB | 7.36 GB |
| 64 | 287 KB | 342 KB | 5.49 GB | 11.1 GB |
| 128 | 305 KB | 357 KB | 6.44 GB | 19.0 GB |

Absorbing a child aggregate and adding one dependency, the round a mempool node runs:

| child covers | flat prove, run 1 | flat prove, run 2 | absorb and add one, run 1 | absorb and add one, run 2 |
|---|---|---|---|---|
| 8 | 133 ms | 508 ms | 295 ms | 395 ms |
| 16 | 223 ms | 856 ms | 273 ms | 385 ms |
| 32 | 417 ms | 1,558 ms | 237 ms | 415 ms |

What moved between the runs, and what did not:

| | Run 1 | Run 2 |
|---|---|---|
| Flat proving crosses `AGGREGATION_INTERVAL` (1,000 ms) at | between 32 and 64 dependencies | 8 dependencies |
| Absorb round, slowest of the three | 295 ms | 415 ms |
| Proof size growth from 1 to 128 dependencies | 1.35x | 1.35x |
| Verification from 1 to 64 dependencies | 3 to 8 ms | 11 to 34 ms |
| Absorb cost tracks child size | no | no |

Proving a Keccak-256 hash chain in-circuit is what makes run 2 slower to prove and
heavier in memory. The three properties the spec contributions state as requirements
held in both runs.

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
