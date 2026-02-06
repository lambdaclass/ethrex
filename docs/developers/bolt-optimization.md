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
sudo ln -sf /usr/bin/llvm-bolt-19 /usr/local/bin/llvm-bolt
sudo ln -sf /usr/bin/perf2bolt-19 /usr/local/bin/perf2bolt
sudo ln -sf /usr/bin/merge-fdata-19 /usr/local/bin/merge-fdata
sudo ln -sf /usr/lib/llvm-19/lib/libbolt_rt_instr.a /usr/local/lib/libbolt_rt_instr.a
```

#### Option 2: Latest from apt.llvm.org (BOLT 22+)
```bash
wget -qO- https://apt.llvm.org/llvm-snapshot.gpg.key | sudo tee /etc/apt/trusted.gpg.d/apt.llvm.org.asc
echo "deb http://apt.llvm.org/unstable/ llvm-toolchain main" | sudo tee /etc/apt/sources.list.d/llvm.list
sudo apt update && sudo apt install bolt-22 libbolt-22-dev
sudo ln -sf /usr/bin/llvm-bolt-22 /usr/local/bin/llvm-bolt
sudo ln -sf /usr/bin/perf2bolt-22 /usr/local/bin/perf2bolt
sudo ln -sf /usr/bin/merge-fdata-22 /usr/local/bin/merge-fdata
sudo ln -sf /usr/lib/llvm-22/lib/libbolt_rt_instr.a /usr/local/lib/libbolt_rt_instr.a
```

## Quick Start

### One command (fully automated)

```bash
make bolt-full    # build → instrument → profile → optimize → verify
make bolt-bench   # benchmark baseline vs BOLT-optimized
```

This uses the built-in ERC20 benchmark fixture (`l2-1k-erc20.rlp`, 1,110 blocks,
~1.5M transactions). Prerequisites are validated automatically — if `llvm-bolt` is
missing, the platform is wrong, or the fixture file hasn't been pulled from Git LFS,
the build will fail with a clear error message.

**Note:** The fixture `l2-1k-erc20.rlp` is stored in Git LFS. If you see an error
about the file being missing or empty, run `git lfs pull` first.

### Step by step

If you want to run each step individually or use a custom profiling workload:

```bash
# 1. Build a BOLT-compatible binary (~2-3 min with fat LTO)
make build-bolt

# 2. Create an instrumented binary
make bolt-instrument

# 3a. Profile with built-in benchmark (automated)
make bolt-profile

# 3b. OR profile with a custom workload (manual)
#     Use Ctrl-C (SIGINT) to stop — BOLT needs graceful shutdown to flush data.
./ethrex-instrumented --network mainnet --syncmode snap --datadir /tmp/bolt-data
# Let it run 60-90 seconds, then Ctrl-C

# 4. Optimize using collected profiles
make bolt-optimize

# 5. Verify and benchmark
make bolt-verify
make bolt-bench
```

### Alternative: Using perf Instead of Instrumentation

If you prefer `perf` (lower runtime overhead, but needs kernel perf access):

```bash
make build-bolt

perf record -e cycles:u -j any,u -o perf.data -- \
    target/release-bolt/ethrex \
        --network fixtures/genesis/perf-ci.json \
        --datadir /tmp/bolt-data \
        import fixtures/blockchain/l2-1k-erc20.rlp

make bolt-perf2bolt
make bolt-optimize
```

## Example: Full Walkthrough with Expected Output

This section shows what each step looks like when it succeeds, so you can tell
if something went wrong.

### Step 1: Build (`make build-bolt`)

```
$ make build-bolt
CXXFLAGS='-fno-reorder-blocks-and-partition' cargo build --profile release-bolt --config .cargo/bolt.toml
   Compiling ethrex-rlp v9.0.0
   ...
   Compiling ethrex v9.0.0
    Finished `release-bolt` profile [optimized + debuginfo] target(s) in 2m 20s
```

Build takes ~2-3 minutes due to fat LTO. Output binary: `target/release-bolt/ethrex`.

**If you see this instead:** `ERROR: BOLT requires x86_64` or `ERROR: llvm-bolt not found` —
fix your platform or install BOLT (see [Installing BOLT](#installing-bolt)).

### Step 2: Instrument (`make bolt-instrument`)

```
$ make bolt-instrument
BOLT-INFO: Target architecture: x86_64
BOLT-INFO: enabling relocation mode
BOLT-WARNING: split function detected on input : malloc_init_hard_a0_locked.cold. The support is limited in relocation mode
BOLT-WARNING: Failed to analyze 1580 relocations
BOLT-INSTRUMENTER: Number of function descriptors: 25271
BOLT-INSTRUMENTER: Number of branch counters: 403378
BOLT-INSTRUMENTER: Total number of counters: 790788
BOLT-INFO: output linked against instrumentation runtime library
Instrumented binary created: ethrex-instrumented
```

The `split function detected` and `Failed to analyze N relocations` warnings are **normal** —
they come from jemalloc's `.cold` symbols and are harmless. What matters is:
- "output linked against instrumentation runtime library" appears
- The `ethrex-instrumented` binary is created

**If you see:** `BOLT-ERROR: library not found: /usr/local/lib/libbolt_rt_instr.a` —
create the symlink: `sudo ln -sf /usr/lib/llvm-19/lib/libbolt_rt_instr.a /usr/local/lib/libbolt_rt_instr.a`

**If you see:** `BOLT-ERROR: parent function not found for ...some_function.../1(*2)` —
a Rust function name contains `warm` or `cold`, or a closure is too complex for BOLT.
See [Troubleshooting](#parent-function-not-found--split-function-detected-errors).

### Step 3: Profile (`make bolt-profile`)

```
$ make bolt-profile
Profiling with fixtures/blockchain/l2-1k-erc20.rlp (this may take a few minutes)...
2026-02-06T21:21:03Z  INFO Importing blocks from file path=fixtures/blockchain/l2-1k-erc20.rlp
2026-02-06T21:21:12Z  INFO [SYNCING] 40% of batch processed
2026-02-06T21:21:21Z  INFO [SYNCING] 60% of batch processed
2026-02-06T21:21:28Z  INFO [SYNCING] 80% of batch processed
2026-02-06T21:22:01Z  INFO Import completed blocks=1110 seconds=79.341
Profile data collected:
-rw-r--r-- 1 admin admin 15M Feb  6 21:22 /tmp/bolt-profiles/prof.12345.fdata
```

The instrumented binary is ~4x slower than normal (it's counting every branch).
Expect ~80 seconds for the 1,110 blocks. The profile file should be **10-20 MB**.

**If the profile directory is empty:** the binary was killed with SIGTERM instead of
exiting gracefully. For long-running workloads (snap sync), use `Ctrl-C` or
`kill -INT <pid>` — BOLT's runtime flushes profile data via an `atexit` handler.

### Step 4: Optimize (`make bolt-optimize`)

```
$ make bolt-optimize
BOLT-INFO: 4504 out of 25589 functions in the binary (17.6%) have non-empty execution profile
BOLT-INFO: ICF folded 360 out of 25851 functions in 4 passes. 0 functions had jump tables.
BOLT-INFO: basic block reordering modified layout of 2851 functions (63.30% of profiled)
BOLT-INFO: program-wide dynostats after all optimizations before SCTC and FOP:

         17340235704 : executed forward branches
          4523681347 : taken forward branches
         ...
         17477374136 : executed forward branches (+0.8%)
           723168777 : taken forward branches (-84.0%)    <-- good: fewer taken branches
         ...
          4617316049 : taken branches (-53.6%)             <-- good: 53% fewer total branches
         ...
```

Key lines to check:
- **"N out of M functions ... have non-empty execution profile"** — should be >10%.
  If 0%, your profile didn't capture the right workload.
- **"taken forward branches (-84%)"** and **"taken branches (-53%)"** — negative
  percentages mean BOLT improved the layout. Higher reductions = better.
- **"ICF folded N functions"** — identical code was deduplicated.

### Step 5: Verify (`make bolt-verify`)

```
$ make bolt-verify
✓ Binary contains BOLT markers
```

### Step 6: Benchmark (`make bolt-bench`)

```
$ make bolt-bench
=== Baseline (3 runs) ===
2026-02-06T21:24:39Z  INFO Import completed blocks=1110 seconds=17.698
2026-02-06T21:24:57Z  INFO Import completed blocks=1110 seconds=17.622
2026-02-06T21:25:14Z  INFO Import completed blocks=1110 seconds=17.583

=== BOLT-optimized (3 runs) ===
2026-02-06T21:25:32Z  INFO Import completed blocks=1110 seconds=17.405
2026-02-06T21:25:49Z  INFO Import completed blocks=1110 seconds=17.385
2026-02-06T21:26:07Z  INFO Import completed blocks=1110 seconds=17.429
```

Compare the `seconds=` values. In this example, BOLT is ~1.4% faster
(17.47s avg vs 17.63s avg). The first run of each set may be slower due
to cold caches — compare runs 2-3 for the cleanest signal.

## Choosing a Representative Workload

The quality of BOLT optimization depends on your profiling workload. Choose workloads that:

1. **Match production usage** - Run the same operations your production binary will execute
2. **Cover hot paths** - Exercise the most frequently executed code
3. **Are long enough** - Run for at least 10-30 seconds to collect meaningful data
4. **Are repeatable** - Use consistent inputs for stable optimization

**Important:** Profile quality directly affects optimization quality. In testing, a profile
collected during snap sync (networking-heavy) produced **0% improvement** on block execution,
while a profile from block import (EVM-heavy) produced **1.4% improvement** on the same
benchmark. Always profile with the workload you want to optimize.

**Available benchmark fixtures:**

| Fixture | Genesis | Blocks | Transactions | Best for |
|---------|---------|--------|-------------|----------|
| `l2-1k-erc20.rlp` | `perf-ci.json` | 1,110 | ~1.5M ERC20 transfers | EVM execution |
| `2000-blocks.rlp` | `perf-ci.json` | 2,004 | ~0 per block | Storage/merkle |

**Examples:**
```bash
# Heavy EVM workload — best for block execution optimization
./ethrex-instrumented \
    --network fixtures/genesis/perf-ci.json \
    --datadir /tmp/bolt-data \
    import fixtures/blockchain/l2-1k-erc20.rlp

# Snap sync — covers networking + state sync paths
./ethrex-instrumented --network mainnet --syncmode snap --datadir /tmp/bolt-data
# Let it run 60-90 seconds, then Ctrl-C
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

### "Parent function not found" / "Split function detected" Errors

BOLT's split-function detection regex matches `.warm` and `.cold` substrings anywhere in
ELF symbol names. Rust's legacy mangling converts `::` to `..`, so function names containing
`warm` or `cold` will trigger false positives:

- `::warm_block` → `..warm_block..` contains `.warm` → BOLT error
- `::cold_start` → `..cold_start..` contains `.cold` → BOLT error

**Fix:** Rename the function to avoid `warm`/`cold` (e.g., `warm_block` → `preheat_block`).

Additionally, closures passed to `std::thread::spawn` produce complex `drop_in_place`
specialization symbols that BOLT can't analyze. **Fix:** Extract the closure body into a
named function and pass it as a function pointer.

The `CXXFLAGS='-fno-reorder-blocks-and-partition'` flag in `build-bolt` prevents C++ dependencies
(e.g., RocksDB) from producing split functions. Remaining jemalloc `.cold` symbols are harmless
warnings that don't cause errors.

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

## Makefile Targets Reference

| Target | Description |
|--------|-------------|
| `make bolt-full` | Full automated workflow: build → instrument → profile → optimize → verify |
| `make bolt-bench` | Benchmark baseline vs BOLT-optimized (3 runs each) |
| `make build-bolt` | Build BOLT-compatible binary with fat LTO and relocations |
| `make bolt-instrument` | Create instrumented binary for profiling |
| `make bolt-profile` | Run instrumented binary with benchmark fixture to collect profiles |
| `make bolt-optimize` | Apply BOLT optimization using collected profiles |
| `make bolt-verify` | Check that optimized binary has BOLT markers |
| `make bolt-perf2bolt` | Convert `perf.data` to BOLT format (alternative to instrumentation) |
| `make bolt-clean` | Remove all BOLT artifacts and profiles |

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

```bash
make bolt-bench
```

This runs 3 iterations each of baseline and BOLT-optimized, importing the ERC20
benchmark blocks. Compare the `seconds=` values in the `Import completed` output.

Also look for:
- Better instruction cache hit rates (via `perf stat`)
- Improved branch prediction accuracy

### Measured Results (ethrex-office-3)

Tested on 128-core AMD EPYC (Debian Trixie), BOLT 19, Rust 1.90.0-nightly.

**Workload:** Import 1,110 blocks containing ~1.5M ERC20 transfer transactions
(2.4-2.9 Ggas/s), using `fixtures/genesis/perf-ci.json` + `fixtures/blockchain/l2-1k-erc20.rlp`.

| Binary | Avg time (5 runs) |
|--------|-------------------|
| Baseline (release-bolt) | 17,706 ms |
| BOLT-optimized | 17,465 ms |
| **Improvement** | **~1.4%** |

BOLT dynostats showed significant branch prediction improvements:
- Taken forward branches: **-84.0%**
- Taken conditional branches: **-50.6%**
- Total taken branches: **-53.6%**

**Note:** The improvement is expected to be higher on a live node with concurrent
networking, mempool processing, and consensus, where I-cache pressure is greater.

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
   - `-Cllvm-args=-hot-cold-split=false` - Prevents LLVM from creating `.cold` fragments that BOLT can't match
   - Loaded only by `make build-bolt` via `--config .cargo/bolt.toml` to avoid affecting normal builds

3. **Build constraints**:
   - RocksDB must build with `-fno-reorder-blocks-and-partition` (set by the `build-bolt` target)
   - Fat LTO (`lto = "fat"` in release-bolt profile) — thin LTO creates `.lto_priv` fragments incompatible with BOLT

4. **Rust source constraints** (for BOLT compatibility):
   - Function names must not contain `warm` or `cold` — BOLT's split-function regex matches `.warm`/`.cold` inside Rust's mangled symbols
   - Closures in `thread::spawn` should be extracted into named function pointers — complex closure symbols confuse BOLT
