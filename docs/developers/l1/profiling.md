# Profiling

This guide covers the profiling tools available for ethrex developers, including CPU profiling and memory profiling.

## CPU Profiling with `pprof`

Ethrex includes built-in CPU profiling via [pprof-rs](https://github.com/tikv/pprof-rs), gated behind the `cpu_profiling` feature flag. When enabled, a profiler starts at boot (1000 Hz sampling) and writes a `profile.pb` file to the current working directory at shutdown.

### Prerequisites

To view the generated profiles you need one of:

- **Go toolchain** (`go tool pprof`) — the standard pprof viewer
- **pprof CLI** — standalone binary from [google/pprof](https://github.com/google/pprof)

Install the standalone CLI:

```bash
go install github.com/google/pprof@latest
```

### Building with CPU profiling

```bash
# Debug build
cargo build -p ethrex --features cpu_profiling

# Release build (recommended for realistic profiles)
cargo build -p ethrex --release --features cpu_profiling
```

The `cpu_profiling` feature is opt-in (not in `default`) so normal builds are unaffected.

### Collecting a profile

1. Start the node as usual:

   ```bash
   ./target/release/ethrex --authrpc.jwtsecret ./secrets/jwt.hex --network holesky
   ```

   You should see this log line near startup:

   ```
   CPU profiling enabled (1000 Hz), will write profile.pb at shutdown
   ```

2. Let the node run through the workload you want to profile.

3. Stop the node with `Ctrl+C` or `SIGTERM`. The file `profile.pb` will be written to the current working directory and the shutdown logs will include:

   ```
   CPU profile written to profile.pb
   ```

### Analyzing the profile

#### Interactive web UI

```bash
go tool pprof -http=:8080 profile.pb
```

This opens a browser with flame graphs, call graphs, top functions, and source annotations.

#### Terminal top functions

```bash
go tool pprof profile.pb
# then at the (pprof) prompt:
(pprof) top 20
(pprof) top 20 -cum
```

#### Flame graph (SVG)

```bash
go tool pprof -svg profile.pb > flamegraph.svg
```

#### Focus on a specific function

```bash
go tool pprof -http=:8080 -focus=execute_block profile.pb
```

### Tips

- **Use release builds** for profiling. Debug builds have very different performance characteristics due to missing optimizations and extra debug assertions.
- **Profile with `release-with-debug`** if you want accurate profiles with full symbol names. This gives optimized code with debug symbols:
  ```bash
  cargo build -p ethrex --profile release-with-debug --features cpu_profiling
  ```
- **Combine with jemalloc** — the `cpu_profiling` feature is orthogonal to `jemalloc` and `jemalloc_profiling`. You can enable both:
  ```bash
  cargo build -p ethrex --release --features cpu_profiling,jemalloc
  ```
- **Sampling rate** — the profiler samples at 1000 Hz (once per millisecond). This is high enough to get good resolution without significant overhead.
- **File location** — `profile.pb` is written to whichever directory you run the binary from. If you want it elsewhere, `cd` to that directory before starting the node, or move the file after shutdown.

## Memory Profiling with jemalloc

Ethrex supports memory profiling through jemalloc, gated behind the `jemalloc_profiling` feature flag. This enables jemalloc's built-in heap profiling (`prof:true`) and exposes `/debug/pprof/allocs` and `/debug/pprof/allocs/flamegraph` RPC endpoints for on-demand heap dumps.

### Building with memory profiling

```bash
cargo build -p ethrex --release --features jemalloc_profiling
```

> **Note:** `jemalloc_profiling` implies the `jemalloc` feature, so you don't need to specify both.

### External memory profilers

You can also use external tools with the `jemalloc` feature (without `jemalloc_profiling`):

#### Bytehound

Requires [Bytehound](https://github.com/koute/bytehound) and jemalloc installed on the system.

```bash
cargo build -p ethrex --release --features jemalloc

export MEMORY_PROFILER_LOG=warn
LD_PRELOAD=/path/to/libbytehound.so:/path/to/libjemalloc.so ./target/release/ethrex [ARGS]
```

#### Heaptrack (Linux only)

Requires [Heaptrack](https://github.com/KDE/heaptrack) and jemalloc installed on the system.

```bash
cargo build -p ethrex --release --features jemalloc

LD_PRELOAD=/path/to/libjemalloc.so heaptrack ./target/release/ethrex [ARGS]
heaptrack_print heaptrack.ethrex.<pid>.gz > heaptrack.stacks
```

## Profiling with Samply

[Samply](https://github.com/mstange/samply) is a sampling CPU profiler that works on macOS and Linux and produces profiles viewable in the Firefox Profiler.

```bash
cargo build -p ethrex --profile release-with-debug
samply record ./target/release-with-debug/ethrex [ARGS]
```

This will open the Firefox Profiler UI in your browser when the process exits.

## Profiling with hotpath

[hotpath](https://hotpath.rs) is a feature-gated Rust profiler that measures function timing and per-function allocation, with zero cost when disabled: with the feature off, the `#[cfg_attr]`-gated instrumentation is never emitted, so there is no dependency, no codegen, and no runtime difference from a build without hotpath at all.

### Feature flags

Two cargo features on the `ethrex` binary crate control hotpath:

| Feature         | What it does                                          |
|-----------------|--------------------------------------------------------|
| `hotpath`       | Function timing (base feature; `hotpath-alloc` builds on it) |
| `hotpath-alloc` | Adds per-function allocation tracking (bytes/count)    |

Neither is in `default`.

### Building and running

Always scope the build to the `ethrex` package with `-p ethrex`:

```bash
cargo build -p ethrex --features hotpath
# or
cargo build -p ethrex --features hotpath-alloc
```

Run the node as usual. On graceful shutdown (`Ctrl+C` / `SIGTERM`), a `[hotpath]` report prints to stdout with tables for timing, allocation, and per-thread breakdowns, depending on which features are enabled.

For a quick dev run there is a Makefile target that boots the node in dev mode with the in-memory engine and the profiler enabled:

```bash
make dev-hotpath                                     # timing + allocations (default)
make dev-hotpath HOTPATH_FEATURES=hotpath            # timing only
```

### Live TUI dashboard

For real-time monitoring instead of the shutdown report, install the standalone hotpath CLI (`cargo install hotpath --features tui`) and run `hotpath console` in a second terminal while the node runs with any `hotpath` feature enabled. The instrumented process exposes a small HTTP server (compiled in by the `hotpath` feature) that the TUI reads live; no extra ethrex configuration is required.

Useful environment variables (see [hotpath.rs](https://hotpath.rs) for the full list): `HOTPATH_REPORT` (which report sections to print), `HOTPATH_ALLOC_METRIC=bytes|count` (allocation metric unit), `HOTPATH_OUTPUT_PATH` (write the report to a file instead of stdout), and percentile/limit variables for trimming the timing tables.

### Currently instrumented functions

This pass instruments the synchronous hot paths, covering the three
block-processing phases on the live (`add_block`) path — execution,
merkleization (state-root), and commit — plus one level of decomposition
under each:

Execution:
- `Blockchain::execute_block` (`crates/blockchain/blockchain.rs`) — full execution phase (pre-validation + block execution + state-transition build + post-validation)
- `Blockchain::execute_block_pipeline` (`crates/blockchain/blockchain.rs`) — execution (import/sync path; fuses exec + merkleization across parallel threads)
- `Evm::execute_block` (`crates/vm/backends/mod.rs`) — pure block execution (the transaction loop), isolated from surrounding validation
- `Evm::get_state_transitions` (`crates/vm/backends/mod.rs`) — builds the `AccountUpdate` set from the VM cache (the exec→merkleization bridge)
- `VM::execute` (`crates/vm/levm/src/vm.rs`) — per-transaction EVM execution

Merkleization (state-root):
- `Store::apply_account_updates_batch` (`crates/storage/store.rs`) — merkleization phase entry
- `Store::apply_account_updates_from_trie_batch` (`crates/storage/store.rs`) — the trie-batch core beneath it
- `Trie::collect_changes_since_last_hash` (`crates/common/trie/trie.rs`) — per-trie node hashing (fires once for the state trie and once per modified storage trie)

Commit:
- `Blockchain::store_block` (`crates/blockchain/blockchain.rs`) — commit phase

> **Commit is enqueue, not disk I/O.** `store_block` → `store_block_updates` → `apply_updates` hands the batch to a background persist worker (`store.rs`), so the `store_block` number measures the hand-off, not the actual RocksDB write. The write happens off-thread on the persist worker (uninstrumented, and subject to the multi-thread caveat below).

`execute_block` and `execute_block_pipeline` are alternate paths: block import and full-sync use the pipeline, so a report from `ethrex import` shows `execute_block_pipeline` but not `execute_block`. Only one of the two appears in a given run. On the pipeline path, execution and merkleization run concurrently on scoped threads, so they surface as the single `execute_block_pipeline` measurement rather than split out; the separate `Store::apply_account_updates_batch` / `store_block` split is observable on the live `add_block` path.

### Async / multi-thread allocation caveat

hotpath's allocation tracking is thread-local and assumes a `current_thread` tokio runtime. ethrex runs a multi-thread `#[tokio::main]` runtime, so allocation counts attributed to `async fn`s are unreliable: allocations performed on a spawned worker thread are not attributed back to the `async fn` that awaited the work. Timing is not affected by this and works correctly on any runtime.

All currently instrumented functions are synchronous, so their allocation numbers under `hotpath-alloc` are meaningful. If you instrument an `async fn` in the future, prefer timing over allocation tracking for it.

### jemalloc interaction

ethrex's default global allocator is jemalloc (via the `jemalloc` feature). Under `hotpath-alloc`, ethrex's own `#[global_allocator]` is cfg-disabled and allocations instead route through `hotpath::CountingAllocator<Jemalloc>`, which counts each allocation and then delegates to jemalloc. `malloc_conf` tuning (`background_thread`, decay settings, etc.) still applies, since jemalloc remains the backing allocator, but the counting wrapper adds per-allocation overhead. Treat `hotpath-alloc` runs as useful for allocation *attribution* (which function allocates), not as representative of production allocator latency/throughput.

If jemalloc is disabled or the target is `msvc`, the backing allocator falls back to `std::alloc::System`. At startup, the log line

```
Global allocator: hotpath CountingAllocator<jemalloc|system>
```

reports which backing allocator is active.

### Follow-up / not yet instrumented

The following are documented targets for future hotpath instrumentation, not covered by this pass:

- Deeper trie internals (individual node RLP encoding / per-node hashing beneath `Trie::collect_changes_since_last_hash`; these run per node, so instrument only if the aggregate trie number points there — per-node `measure` overhead is high)
- The actual RocksDB persist worker (the off-thread write behind the commit enqueue)
- RLP
- RPC
- The levm opcode dispatch loop / per-opcode handlers

For the opcode loop specifically: the existing `perf_opcode_timings` feature (`ethrex-vm/perf_opcode_timings`) already provides opcode-level timing. Any future hotpath work on the opcode loop must reconcile with `perf_opcode_timings` rather than duplicate it.

### Guardrail: never enable hotpath workspace-wide

Always enable hotpath scoped to the binary crate:

```bash
cargo build -p ethrex --features hotpath
```

Never do:

```bash
cargo build --workspace --features hotpath
```

A `--workspace --features hotpath` invocation unifies the `hotpath` feature into every workspace member that has a matching feature name, including transitively-built crates. In particular it defeats guest isolation for the zkVM guest program (`ethrex-guest-program`), which must never depend on `hotpath`. This was verified: `cargo tree --workspace --features hotpath` shows a `hotpath` reference for every hotpath-enabled crate, while `cargo tree -p ethrex --features hotpath` keeps it scoped to the intended crates. The guest crate itself does not define a `hotpath` feature, so a properly `-p`-scoped build can never leak into it, but a `--workspace` build risks pulling in the dependency across the board.

No CI job today passes `--features` together with `--workspace`, so this is not currently an issue in practice. Any new CI job that builds with hotpath enabled must scope it with `-p ethrex`, not `--workspace`.

## Feature flags summary

| Feature              | What it does                                     | Platform   |
|----------------------|--------------------------------------------------|------------|
| `cpu_profiling`      | Built-in pprof CPU profiling, writes `profile.pb`| Linux/macOS|
| `jemalloc`           | Use jemalloc allocator (enables external profilers)| Linux/macOS|
| `jemalloc_profiling` | jemalloc heap profiling + RPC endpoint           | Linux/macOS|
| `hotpath`            | Function timing report on shutdown               | Linux/macOS|
| `hotpath-alloc`      | Adds per-function allocation tracking to hotpath | Linux/macOS|
