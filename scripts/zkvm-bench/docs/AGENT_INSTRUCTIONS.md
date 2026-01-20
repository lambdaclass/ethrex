# zkVM Optimization Agent Instructions

You are an autonomous agent tasked with optimizing the ethrex zkVM guest program for minimal cycle count and proving time. Your goal is to iteratively identify and fix performance bottlenecks.

## Context Files

Before starting, read these files to understand the codebase:
- `scripts/zkvm-bench/docs/zkvm_landscape.md` — Patch registry, crypto operation mapping, known gaps
- `scripts/zkvm-bench/docs/zkvm_optimization_workflow.md` — Optimization principles and workflow details
- `scripts/zkvm-bench/README.md` — Benchmarking tools documentation

## Working Directory

Always operate from the repository root:
```bash
cd /path/to/ethrex
```

## Optimization Workflow

### Phase 1: Establish Baseline

1. **Generate input for a representative block:**
   ```bash
   make -C scripts/zkvm-bench input BLOCK=23769082 RPC_URL=$RPC_URL
   ```

2. **Build and profile both backends:**
   ```bash
   # ZisK baseline
   make -C scripts/zkvm-bench bench ZKVM=zisk BLOCK=23769082 TITLE="baseline"

   # SP1 baseline
   make -C scripts/zkvm-bench bench ZKVM=sp1 BLOCK=23769082 TITLE="baseline"
   ```

3. **Save baseline profiles:**
   ```bash
   # (Optional) If you want a specific "baseline.txt" copy, though the timestamped/titled one is usually sufficient.
   cp scripts/zkvm-bench/profiles/zisk/stats_*_baseline.txt scripts/zkvm-bench/profiles/zisk/baseline.txt
   ```

4. **Convert to JSON for analysis:**
   ```bash
   make -C scripts/zkvm-bench to-json FILE=scripts/zkvm-bench/profiles/zisk/baseline.txt
   ```

### Phase 2: Analyze Bottlenecks

Examine the profile output and identify optimization targets:

#### High-Priority Targets (fix these first):

1. **Unpatched crypto operations** — Look for these patterns in TOP COST FUNCTIONS:
   - `tiny_keccak::Keccak::update` instead of `syscall_keccak_f` → Keccak not using precompile
   - `ark_bn254::` anything → BN254 not using substrate-bn patch
   - `sha2::sha256::compress` → SHA256 not using precompile
   - `k256::` high cycles → secp256k1 patch may be missing
   - `malachite::` → MODEXP not using precompile (ZisK has one, others don't)

2. **Memory operations** — High cost in:
   - `compiler_builtins::mem::memcpy` → Excessive cloning/copying
   - `alloc::` functions → Unnecessary allocations

3. **Serialization overhead:**
   - `rkyv::` or `rlp::` functions → Encoding/decoding overhead

#### Check Patch Utilization:

Reference the crypto operation matrix in `zkvm_landscape.md`:

| Operation | ZisK | SP1 | RISC0 |
|-----------|------|-----|-------|
| ECRECOVER | ✅ k256 | ✅ k256 | ✅ k256 |
| Keccak-256 | ✅ tiny-keccak | ✅ tiny-keccak | ❌ unpatched |
| BN254 ECADD | ✅ substrate-bn | ❌ ark_bn254 | ❌ ark_bn254 |
| BN254 ECMUL | ✅ substrate-bn | ✅ substrate-bn | ❌ ark_bn254 |
| BLS12-381 | ✅ sp1_bls12_381 | ❌ unpatched | ❌ unpatched |
| MODEXP | ✅ ziskos::modexp | ❌ malachite | ❌ malachite |

### Phase 3: Implement Optimization

Based on your analysis, implement ONE optimization at a time:

#### Option A: Add/Fix Patch

1. Check guest program's `Cargo.toml`:
   ```
   crates/l2/prover/src/guest_program/src/{zisk,sp1}/Cargo.toml
   ```

2. Verify patch is declared in `[patch.crates-io]` section

3. Check patch tag exists and version matches

#### Option B: Code Optimization

1. Locate the hot function in the codebase:
   ```bash
   # Search for function
   rg "function_name" crates/
   ```

2. Common optimizations:
   - Replace `.clone()` with references where possible
   - Use `&[u8]` instead of `Vec<u8>` for read-only data
   - Avoid intermediate allocations in hot paths
   - Use `MaybeUninit` for large stack arrays
   - Batch operations instead of per-item processing

3. **Do NOT:**
   - Add unnecessary abstractions
   - Optimize cold paths
   - Break existing functionality
   - Change public APIs without necessity

### Phase 4: Validate Optimization

1. **Rebuild the guest program:**
   ```bash
   make -C scripts/zkvm-bench build ZKVM=zisk
   ```

2. **Profile again (with description):**
   ```bash
   make -C scripts/zkvm-bench profile ZKVM=zisk BLOCK=23769082 TITLE="optimization_name"
   ```

3. **Compare with baseline:**
   ```bash
   make -C scripts/zkvm-bench compare \
     BASELINE=scripts/zkvm-bench/profiles/zisk/baseline.txt \
     CURRENT=scripts/zkvm-bench/profiles/zisk/stats_*_optimization_name.txt
   ```

4. **Evaluate results:**
   - **IMPROVEMENT**: Total steps decreased → Keep the change, update baseline
   - **REGRESSION**: Total steps increased → Revert the change, try different approach
   - **NO CHANGE**: Steps unchanged → Change didn't affect hot path, may still be valuable

5. **If improvement, update baseline:**
   ```bash
   cp scripts/zkvm-bench/profiles/zisk/stats_*_optimization_name.txt scripts/zkvm-bench/profiles/zisk/baseline.txt
   ```

### Phase 5: Report

1. Update `scripts/zkvm-bench/logbook.md` with a new entry for the optimization.
2. Include the Date, ID, Description, Impact, Result, and Commit hash (if available).
3. Create a detailed report in `scripts/zkvm-bench/reports/YYYYMMDD_optimization_name.md` if the optimization is significant or requires detailed analysis.

You can read previous logs to take advantage of previous iterations.

### Phase 6: Repeat

1. Go back to Phase 2 with the new baseline
2. Continue until:
   - Top functions are all precompile syscalls (optimal)
   - Remaining hotspots are unavoidable (e.g., core VM execution)
   - Diminishing returns (< 1% improvement per iteration)

## Decision Framework

### When to Optimize Code vs. Patches

| Symptom | Action |
|---------|--------|
| Crypto function in top 10 | Check if patch exists, add/fix it |
| `memcpy` > 15% | Look for excessive cloning in hot paths |
| Serialization > 10% | Consider lazy deserialization or caching |
| Single function > 20% | Deep dive into that function's implementation |

### Optimization Priority

1. **Patches** — 100-1000x improvement for crypto ops
2. **Algorithm changes** — 10-100x for better complexity
3. **Memory layout** — 2-10x for cache efficiency
4. **Micro-optimizations** — 1.1-2x for hot loops

### Known Limitations

- **SP1 ECADD**: Cannot use substrate-bn due to GasMismatch bug (see `precompiles.rs:814`)
- **RISC0 Keccak/BLS**: Patches require "unstable" feature, not production-ready
- **ZisK P256**: No patch exists for P256VERIFY precompile

## Commands Reference

### Using Makefile (Recommended)

```bash
# Full workflow with description
make -C scripts/zkvm-bench bench ZKVM=zisk BLOCK=23769082 RPC_URL=$RPC TITLE="my_optimization"

# Individual steps
make -C scripts/zkvm-bench input BLOCK=23769082 RPC_URL=$RPC
make -C scripts/zkvm-bench build ZKVM=zisk
make -C scripts/zkvm-bench profile ZKVM=zisk BLOCK=23769082 TITLE="my_optimization"
make -C scripts/zkvm-bench compare BASELINE=a.txt CURRENT=b.txt

# Utilities
make -C scripts/zkvm-bench list-inputs
make -C scripts/zkvm-bench list-profiles ZKVM=zisk
make -C scripts/zkvm-bench to-json FILE=stats.txt
```

### Direct Script Usage (Advanced)

The scripts are located in `scripts/zkvm-bench/bin/`.

**profile-zisk.sh Arguments:**
```bash
bin/profile-zisk.sh <input_file> [output_dir] [top_roi] [description] [elf_path]
```

**Examples:**
```bash
# Basic profile
scripts/zkvm-bench/bin/profile-zisk.sh scripts/zkvm-bench/inputs/block.bin

# With description and custom ROI
scripts/zkvm-bench/bin/profile-zisk.sh scripts/zkvm-bench/inputs/block.bin scripts/zkvm-bench/profiles/zisk 50 "optimization_attempt_1"
```

**Description Sanitization:**
Descriptions are automatically sanitized (lowercase, spaces to underscores, alphanumeric only).
- `"My Optimization #1"` → `my_optimization_1`

### Direct ZisK Emulator

For detailed analysis beyond the scripts:
```bash
ziskemu -e <ELF> -i <INPUT> -X -S -D -T 50
```

## Safety Rules

1. **Always create a git branch** before making changes
2. **One optimization per commit** for easy bisection
3. **Never skip validation** — always compare before/after
4. **Preserve correctness** — a faster wrong answer is worthless
5. **Document findings** — update `zkvm_landscape.md` and `logbook.md` with new discoveries
