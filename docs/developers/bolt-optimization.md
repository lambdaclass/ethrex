# BOLT+PGO Optimization Guide

This guide explains how to use LLVM BOLT (Binary Optimization and Layout Tool) to optimize ethrex binaries for production performance.

## Overview

BOLT is a post-link optimizer that rearranges binary code layout based on runtime profiling data. It can improve performance by **2-15%** through better instruction cache utilization and branch prediction, with no source code changes required.

**Key benefits:**
- Profile-guided code layout optimization
- Improved instruction cache hit rate
- Better branch prediction
- Function reordering for better locality
- Works on final binaries (post-link)

## Requirements

- **Platform:** Linux x86_64 (ARM64 currently unsupported due to LLVM bug)
- **LLVM version:** 19+ (tested with BOLT 19 and 22)
- **Tools needed:** `llvm-bolt`, `perf2bolt`, `merge-fdata`, `libbolt_rt_instr.a`

### Installing BOLT

#### Option 1: Debian Trixie (BOLT 19)
```bash
sudo apt install bolt-19 libbolt-19-dev
sudo ln -sf /usr/lib/llvm-19/lib/libbolt_rt_instr.a /usr/lib/libbolt_rt_instr.a
```

#### Option 2: Latest from apt.llvm.org (BOLT 22+)
```bash
wget -qO- https://apt.llvm.org/llvm-snapshot.gpg.key | sudo tee /etc/apt/trusted.gpg.d/apt.llvm.org.asc
echo "deb http://apt.llvm.org/unstable/ llvm-toolchain main" | sudo tee /etc/apt/sources.list.d/llvm.list
sudo apt update && sudo apt install bolt-22 libbolt-22-dev
sudo ln -sf /usr/lib/llvm-22/lib/libbolt_rt_instr.a /usr/lib/libbolt_rt_instr.a
```

## Quick Start

### Method 1: Using Makefile (Recommended)

This is the simplest workflow for BOLT optimization:

```bash
# 1. Build a BOLT-compatible binary
make build-bolt

# 2. Instrument the binary for profiling
make bolt-instrument

# 3. Run the instrumented binary with representative workload
./ethrex-instrumented <your-workload-args>
# Profile data is written to /tmp/bolt-profiles/prof.<pid>.fdata

# 4. Optimize using collected profiles
make bolt-optimize

# 5. Use the optimized binary
./ethrex-bolt-optimized
```

### Method 2: Using perf (Alternative)

If you prefer using Linux `perf` for profiling:

```bash
# 1. Build a BOLT-compatible binary
make build-bolt

# 2. Profile with perf
perf record -e cycles:u -j any,u -o perf.data -- target/release-bolt/ethrex <workload>

# 3. Convert perf data to BOLT format
make bolt-perf2bolt

# 4. Optimize using collected profiles
make bolt-optimize
```

## Understanding the Workflow

### Step 1: Build with BOLT Compatibility

The `build-bolt` target builds the binary with special flags:

```bash
make build-bolt
```

This sets:
- `CXXFLAGS='-fno-reorder-blocks-and-partition'` - Prevents RocksDB function splitting that confuses BOLT
- `--emit-relocs` linker flag - Preserves relocation information needed by BOLT
- `-Wl,-q` - Quick relocations mode
- `-Cforce-frame-pointers=yes` - Better profiling accuracy
- `debug = 1` in profile - Keeps symbols for BOLT analysis

**Output:** `target/release-bolt/ethrex`

### Step 2: Collect Profile Data

Two methods are available:

#### A. BOLT Instrumentation (Recommended)
```bash
make bolt-instrument
./ethrex-instrumented <representative-workload>
```

This creates a specially instrumented binary that collects detailed execution data in `/tmp/bolt-profiles/prof.<pid>.fdata`.

**Advantages:**
- More accurate BOLT-specific profiling
- No kernel perf setup required
- Captures all data BOLT needs

#### B. Linux perf
```bash
perf record -e cycles:u -j any,u -o perf.data -- target/release-bolt/ethrex <workload>
make bolt-perf2bolt
```

**Advantages:**
- Standard Linux profiling tool
- Can use existing perf workflows
- Lower runtime overhead

### Step 3: Optimize the Binary

```bash
make bolt-optimize
```

This applies BOLT optimizations:
- `-reorder-blocks=ext-tsp` - Advanced block reordering algorithm
- `-reorder-functions=cdsort` - Function reordering by call density
- `-split-functions` - Separate hot/cold code paths
- `-split-all-cold` - Move cold code out of hot paths
- `-split-eh` - Separate exception handling code
- `-dyno-stats` - Display optimization statistics

**Output:** `ethrex-bolt-optimized`

### Step 4: Verify Optimization

```bash
make bolt-verify
```

This confirms the binary was processed by BOLT by checking for the `.note.bolt_info` section.

## Choosing a Representative Workload

The quality of BOLT optimization depends on your profiling workload. Choose workloads that:

1. **Match production usage** - Run the same operations your production binary will execute
2. **Cover hot paths** - Exercise the most frequently executed code
3. **Are long enough** - Run for at least 10-30 seconds to collect meaningful data
4. **Are repeatable** - Use consistent inputs for stable optimization

**Examples:**
```bash
# Sync from a known network (snap sync covers the hot paths)
./ethrex-instrumented --network mainnet --syncmode snap --datadir /tmp/bolt-data

# Import blocks from an RLP chain file
./ethrex-instrumented --network mainnet --datadir /tmp/bolt-data import blocks.rlp

# Run in dev mode with a load test (in a separate terminal: make load-test)
./ethrex-instrumented --dev --datadir /tmp/bolt-data
```

## Advanced: PGO + BOLT Combined

For maximum optimization, combine Profile-Guided Optimization (PGO) with BOLT:

```bash
# 1. Build with PGO instrumentation
make pgo-full-build

# 2. Run profiling workload
./target/release/ethrex <workload>

# 3. Build PGO-optimized binary with BOLT support
CXXFLAGS='-fno-reorder-blocks-and-partition' cargo pgo bolt build --with-pgo

# 4. Instrument for BOLT
make bolt-instrument

# 5. Run workload again
./ethrex-instrumented <workload>

# 6. Apply BOLT optimization
make bolt-optimize
```

This combines:
- **PGO:** Compiler optimizations based on runtime data (function inlining, branch weights)
- **BOLT:** Post-link binary layout optimization

Expected combined improvement: **5-20%** depending on workload.

## Troubleshooting

### ARM64 Not Supported
BOLT currently fails on ARM64 Linux with "Undefined temporary symbol .Ltmp0" errors. This is a known LLVM bug. Use x86_64 for now.

### "Split function detected" Warnings
BOLT may warn about split functions (symbols with `.warm` or `.cold` suffixes added by LLVM during function splitting). The `CXXFLAGS='-fno-reorder-blocks-and-partition'` flag in `build-bolt` prevents this for RocksDB. If warnings still appear, they can typically be ignored if the build succeeds.

### No Profile Data Found
If `bolt-optimize` reports no profile data:
```bash
# Check for profile files
ls -la /tmp/bolt-profiles/

# If using perf method, ensure perf.data exists
ls -la perf.data

# Re-run profiling step with longer workload
```

### Binary Won't Run After Optimization
Verify BOLT version compatibility:
```bash
llvm-bolt --version
readelf -p .note.bolt_info ethrex-bolt-optimized
```

## Cleaning Up

Remove all BOLT artifacts:
```bash
make bolt-clean
```

This removes:
- `/tmp/bolt-profiles/` directory
- `ethrex-instrumented` binary
- `ethrex-bolt-optimized` binary
- `perf.data` file

## Performance Validation

After optimization, benchmark to verify improvements:

```bash
# Baseline (regular release build)
hyperfine --warmup 1 --runs 5 'target/release/ethrex <workload>'

# BOLT-optimized
hyperfine --warmup 1 --runs 5 './ethrex-bolt-optimized <workload>'
```

Look for:
- Reduced wall-clock time
- Better instruction cache hit rates (via `perf stat`)
- Improved branch prediction accuracy

## References

### Official Documentation
- [LLVM BOLT Documentation](https://github.com/llvm/llvm-project/blob/main/bolt/README.md)
- [BOLT: A Practical Binary Optimizer (paper)](https://research.facebook.com/publications/bolt-a-practical-binary-optimizer-for-data-centers-and-beyond/)
- [Linux perf documentation](https://perf.wiki.kernel.org/)

### Technical Details
- [BOLT on the Linux Kernel](https://www.phoronix.com/news/Linux-Kernel-BOLT-Experiment)
- [Performance Engineering with Rust](https://nnethercote.github.io/perf-book/)
- [Post-link optimization overview](https://easyperf.net/blog/2019/10/05/Performance-Analysis-Of-MT-apps)

### Rust-Specific Resources
- [cargo-pgo documentation](https://github.com/Kobzol/cargo-pgo)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [Optimizing Rust programs](https://deterministic.space/high-performance-rust.html)

## Implementation Notes

The BOLT setup in ethrex includes:

1. **Build profiles** (`Cargo.toml`):
   - `release-bolt` - Release build with debug symbols for BOLT
   - `release-pgo-bolt` - Combined PGO+BOLT profile

2. **Linker configuration** (`.cargo/bolt.toml`):
   - `--emit-relocs` - Preserves relocations for BOLT rewriting
   - `-Wl,-q` - Quick relocations mode
   - `-Cforce-frame-pointers=yes` - Better stack traces for profiling
   - Loaded only by `make build-bolt` via `--config .cargo/bolt.toml` to avoid affecting normal builds

3. **Build constraints**:
   - RocksDB must build with `-fno-reorder-blocks-and-partition` (set by the `build-bolt` target)
