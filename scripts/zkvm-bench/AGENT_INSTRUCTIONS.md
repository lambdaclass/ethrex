# zkVM Optimization Agent Instructions

You are an autonomous agent tasked with optimizing the ethrex zkVM guest program for minimal cycle count and proving time. Your goal is to iteratively identify and fix performance bottlenecks.

## Context Files

Before starting, read these files to understand the codebase:
- `scripts/zkvm-bench/zkvm_landscape.md` — Patch registry, crypto operation mapping, known gaps
- `scripts/zkvm-bench/zkvm_optimization_workflow.md` — Optimization principles and workflow details
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
   make -C scripts/zkvm-bench bench ZKVM=zisk BLOCK=23769082

   # SP1 baseline
   make -C scripts/zkvm-bench bench ZKVM=sp1 BLOCK=23769082
   ```

3. **Save baseline profiles:**
   ```bash
   cp scripts/zkvm-bench/profiles/zisk/stats_23769082_*.txt scripts/zkvm-bench/profiles/zisk/baseline.txt
   cp scripts/zkvm-bench/profiles/sp1/trace_*.json scripts/zkvm-bench/profiles/sp1/baseline.json
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

2. **Profile again:**
   ```bash
   make -C scripts/zkvm-bench profile ZKVM=zisk BLOCK=23769082
   ```

3. **Compare with baseline:**
   ```bash
   make -C scripts/zkvm-bench compare \
     BASELINE=scripts/zkvm-bench/profiles/zisk/baseline.txt \
     CURRENT=scripts/zkvm-bench/profiles/zisk/stats_23769082_*.txt
   ```

4. **Evaluate results:**
   - **IMPROVEMENT**: Total steps decreased → Keep the change, update baseline
   - **REGRESSION**: Total steps increased → Revert the change, try different approach
   - **NO CHANGE**: Steps unchanged → Change didn't affect hot path, may still be valuable

5. **If improvement, update baseline:**
   ```bash
   cp scripts/zkvm-bench/profiles/zisk/stats_23769082_*.txt scripts/zkvm-bench/profiles/zisk/baseline.txt
   ```

### Phase 5: Report

Create a new folder with a name and timestamp for the implemented optimization, observations that led to it and results. Make a final section summarizing learnings and important information for future optimizations.

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

## Output Format

After each optimization cycle, report:

```
## Optimization Report

### Change
[Brief description of what was changed]

### Files Modified
- path/to/file1.rs
- path/to/file2.rs

### Results

| Metric | Before | After | Change |
|--------|--------|-------|--------|
| Total Steps | X | Y | -Z% |
| Top Function | name (X%) | name (Y%) | ... |

### Analysis
[Why did this help/not help]

### Next Steps
[What to try next based on new profile]
```

## Example Session

```
1. Baseline: 150M steps, top function is tiny_keccak::Keccak::update at 25%
2. Analysis: Keccak should be using precompile but isn't
3. Fix: Verify tiny-keccak patch in guest Cargo.toml
4. Result: 95M steps (-37%), top function now syscall_keccak_f at 18%
5. Next: memcpy is now 22%, investigate cloning in transaction processing
```

## Commands Reference

```bash
# Full workflow
make -C scripts/zkvm-bench bench ZKVM=zisk BLOCK=23769082 RPC_URL=$RPC

# Individual steps
make -C scripts/zkvm-bench input BLOCK=23769082 RPC_URL=$RPC
make -C scripts/zkvm-bench build ZKVM=zisk
make -C scripts/zkvm-bench profile ZKVM=zisk BLOCK=23769082
make -C scripts/zkvm-bench compare BASELINE=a.txt CURRENT=b.txt

# Utilities
make -C scripts/zkvm-bench list-inputs
make -C scripts/zkvm-bench list-profiles ZKVM=zisk
make -C scripts/zkvm-bench to-json FILE=stats.txt

# Direct ziskemu for detailed analysis
ziskemu -e <ELF> -i <INPUT> -X -S -D -T 50
```

## Safety Rules

1. **Always create a git branch** before making changes
2. **One optimization per commit** for easy bisection
3. **Never skip validation** — always compare before/after
4. **Preserve correctness** — a faster wrong answer is worthless
5. **Document findings** — update `zkvm_landscape.md` with new discoveries
