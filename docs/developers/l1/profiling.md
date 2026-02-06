# Profiling

This guide covers the profiling tools available for ethrex developers, including CPU profiling and memory profiling.

## CPU Profiling with `pprof`

Ethrex includes on-demand CPU profiling via [pprof-rs](https://github.com/tikv/pprof-rs), gated behind the `cpu_profiling` feature flag. When enabled, the RPC server exposes a `GET /debug/pprof/profile` endpoint that captures a CPU profile for a configurable duration and returns protobuf data.

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

2. Capture a profile by hitting the endpoint with curl:

   ```bash
   # Default: 30-second capture at 1000 Hz
   curl -o profile.pb "http://localhost:8545/debug/pprof/profile"

   # Custom: 60-second capture at 500 Hz
   curl -o profile.pb "http://localhost:8545/debug/pprof/profile?seconds=60&frequency=500"
   ```

   Query parameters:
   - `seconds` — capture duration (default: 30, max: 300)
   - `frequency` — sampling frequency in Hz (default: 1000)

   Only one capture can run at a time. A concurrent request returns `409 Conflict`.
   Without the feature flag, the endpoint returns `501 Not Implemented`.

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
- **Sampling rate** — the default 1000 Hz (once per millisecond) gives good resolution without significant overhead. Lower it for longer captures if needed.

## Memory Profiling with jemalloc

Ethrex supports memory profiling through jemalloc, gated behind the `jemalloc_profiling` feature flag. This enables jemalloc's built-in heap profiling (`prof:true`) and exposes a `/debug/heap_profile` RPC endpoint for on-demand heap dumps.

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

## Feature flags summary

| Feature              | What it does                                     | Platform   |
|----------------------|--------------------------------------------------|------------|
| `cpu_profiling`      | On-demand pprof CPU profiling via HTTP endpoint  | Linux/macOS|
| `jemalloc`           | Use jemalloc allocator (enables external profilers)| Linux/macOS|
| `jemalloc_profiling` | jemalloc heap profiling + RPC endpoint           | Linux/macOS|
