# Timeline profiling with flux-profiler

This branch (`perf/flux-profiler`) instruments ethrex with
[flux-profiler](https://github.com/gattaca-com/flux/tree/main/crates/flux-profiler),
a cross-process timeline profiler. Instrumented functions write an open and a
close mark into a per-thread shared-memory ring; a separate process reads the
rings and writes a trace with one track per thread on one timeline, viewable in
[Perfetto](https://ui.perfetto.dev) or [magic-trace](https://magic-trace.org).

It is always on. The node publishes its rings at startup and a frame costs one
atomic load until a reader attaches, about 12 ns once one has. Nothing here
ships to `main`; see [Branch policy](#branch-policy).

## Attaching to a running node

Install the reader once, at the revision the workspace pins:

```
cargo install --git https://github.com/gattaca-com/flux --rev 8064a8dbefa19659562d9810e2b80bab7f33b200 flux-profiler
```

With one instrumented node on the host, attach with no arguments; otherwise pass
`--pid`. Stop with Ctrl-C to write the trace, then open the `.fxt` file in
Perfetto.

```
flux-profiler
flux-profiler --pid <pid> --out node.fxt
```

The reader and the node must run on the same host as the same user: frame names
are resolved by reading the node's binary through `/proc/<pid>/maps`, which is
also why attaching only works on Linux. The rings and the pid file live under
the OS user data directory, `$XDG_DATA_HOME/ethrex/shmem/queues` (by default
`~/.local/share/ethrex/shmem/queues`), not under the node's `--datadir`. A node
started by a service manager therefore needs a resolvable `HOME` or
`XDG_DATA_HOME`, or the rings land in `/tmp`.

Useful capture flags:

| Flag | Effect |
|---|---|
| `--duration 1h` | stop and export after that long |
| `--dump-interval 30s` | append completed frames to the file every interval and free them, so memory stays flat and the file is complete at every point |
| `--filter-short-frames 1us` | drop top-level frames shorter than that, which removes idle polls |
| `--max-mem 2GB` | stop and export once retained events exceed this (default 1 GB) |

For continuous capture, run the reader under a service manager with
`--duration 1h` and restart-always: each run writes one segment, and a node
restart is followed by the next run attaching to the new process. Prune the
segment directory with a `tmpfiles` rule. The unit used on the profiling host:

```ini
[Service]
User=admin
Environment=HOME=/home/admin
WorkingDirectory=/home/admin/flux-traces/segments
ExecStart=/bin/bash -c 'exec flux-profiler --pid "$(cat $HOME/.local/share/ethrex/shmem/queues/pid)" --duration 1h --dump-interval 30s --filter-short-frames 1us --max-mem 2GB --out $HOME/flux-traces/segments/ethrex-$(date -u +%%Y%%m%%dT%%H%%M%%SZ).fxt'
Restart=always
RestartSec=10
```

with `e /home/admin/flux-traces/segments - admin admin 7d` in a `tmpfiles.d`
file. Taking the pid from the rings' pid file is deliberate: `pgrep` on the
binary path also matches a tmux or shell wrapper running the same command
line. A second reader can attach to the same rings for an ad-hoc capture while
the unit runs; the rings are multi-consumer.

Only the node path publishes rings. Subcommands such as `import` do not, because
enabling the profiler unlinks the app's previous rings and rewrites its pid
file, which would cut a running node's capture.

## Build features

Both are off by default and add to the plain frames:

| Feature | Adds | Cost per frame |
|---|---|---|
| `alloc-profile` | bytes allocated and freed on the thread, as counters next to each track; wraps the jemalloc global allocator in the profiler's counting allocator | about 4 ns |
| `perf` | instructions, cycles and cache misses per frame via `rdpmc`; needs `kernel.perf_event_paranoid` at 2 or lower; Linux x86-64 only, elsewhere the counters read as zero | about 50 ns |

```
cargo build --release -p ethrex --features alloc-profile
cargo build --release -p ethrex --features perf
```

Use them on a host set up for them. The profiling host runs both: the
counters cost about 50 ns per frame, which at a few thousand frames per block
is well under a millisecond, and they answer whether a slow frame did more work
or stalled. A node meant to mirror production should stay timing only.

## What is framed

Every thread that writes a frame is named, so tracks read as `block_executor`,
`block-pipeline-N`, `block-warmer-N`, `merkle-worker-N`, `rayon-worker-N`,
`tokio-worker-N` (runtime workers and the blocking pool share the prefix),
`store_persist` and `store_flatkeyvalue`. Block import spawns no OS threads per
block; the warmer, trie prefetch and merkleizer run on persistent pools.

Frames come in three tiers. Add to the first two freely; add to the third only
when a capture shows a phase hiding something.

1. **Phases**, once per block or per request: the executor loop body, the
   import pipeline stages (execution, warming, prefetch, trie update, DB
   update), the persist worker stages, the VM's block-level entry points, the
   engine handler's decode, authentication and validation steps, fork choice,
   payload building.
2. **Per transaction**: transaction execution, per-transaction access-list
   validation, system calls, transactions applied while building a payload.
   Only real block execution is framed per transaction. The speculative
   warmer and the mempool prewarmer re-execute transactions through an
   untimed entry point, because their frames outnumbered real work twenty to
   one and said nothing about it; their block-level frames remain.
3. **Leaves**, with judgment. Each thread's ring holds 256k marks and the reader
   reports lost marks when it falls behind; a capture that reports loss is not a
   measurement. Never frame opcodes or per-read state access.

Two mechanisms produce frames, chosen per site:

- `#[timed]` from `flux_profiler` on a synchronous function. It is the default
  and the cheap one. **Never put it on an `async fn`**: the guard would live
  across await points, so the open mark lands on whichever runtime worker
  started the future and the close mark on whichever finished it, and the frame
  is broken. Time the synchronous helpers an async handler calls instead; the
  awaits show up as gaps between them.
- A tracing span bridged into the rings by the layer in `cmd/ethrex/profiler.rs`.
  This is for `async fn`s, where tracing fires enter and exit on every poll on
  the thread that polled, and for the per-block pipeline spans that already
  feed the Prometheus histograms. The layer carries only the spans on its
  allowlist, `BRIDGED_SPANS`; add a name there when adding a bridged span. The
  per-read VM spans are deliberately not listed.

Frame names are the defaults: `crate::module::function` for `#[timed]`, with the
receiver type folded in for methods, and the span name for bridged spans.

### Known gap

On the Amsterdam path, the access-list merkleizer still spawns scoped OS threads
per block for its state and storage shards (`bal_state_shard_N`,
`bal_storage_worker_N`, `storage_shard_N`). Each produces a ring and a track per
block. Pre-Amsterdam blocks do not take that path. Moving those shards onto the
persistent merkle pool is a separate change.

## Branch policy

`perf/flux-profiler` is never merged. flux is a git-only dependency and the
library crates are published to crates.io, and the annotations are not
something `main` should carry. `main` is merged into this branch when the
maintainer decides.

Optimizations found here ship the other way:

1. Open a segment or a capture in Perfetto. Take the widest gap or the fattest
   frame.
2. Fix it on a branch cut from `main`.
3. Merge that branch into `perf/flux-profiler` locally and capture again for
   per-frame before/after numbers. Compare p50 and max over many blocks.
4. Quote the shipping number from clean release builds off `main`; a build that
   carries frames is not comparable to one that does not.
5. Open the PR from the `main`-based branch.

No commit on this branch mixes an annotation with a behavior change.

## Guest programs

The zkVM guest programs do not build on this branch and are out of scope. Guest
cost is emulator cycles, not wall-clock; use the cycle trackers already wired for
SP1, RISC0 and ZisK.
