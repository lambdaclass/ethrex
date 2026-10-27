# zkVM Optimization Workflow

This document outlines the workflow for optimizing the ethrex zkVM guest program. It is designed for both human engineers and autonomous agents to iteratively identify and fix performance bottlenecks.

## Optimization Principles

Unlike host machine optimization (minimize wall-clock time), zkVM optimization focuses on:

1. **Minimize zkVM cycles** - Proving time scales directly with cycle count
2. **Leverage precompiles** - Specialized circuits for expensive operations:
   - Keccak (often >15% cost if unpatched)
   - Secp256k1 (add/dbl operations)
   - BLS12-381 operations
   - SHA256
3. **Reduce memory operations** - memcpy is expensive (often >15% cost)
4. **Avoid unnecessary allocations** - Cloning, RLP encoding overhead
5. **No branch prediction penalty** - Control flow is cheaper than on CPUs

`zkvm_landscape.md` contains more information on this topic.

## Patch Utilization Context

Patches replace standard crypto crates with zkVM-optimized versions that call precompiles. This is the single most impactful optimization.

Reference the crypto operation matrix in `zkvm_landscape.md` for details, but here is a quick summary:

| Operation | ZisK | SP1 | RISC0 |
|-----------|------|-----|-------|
| ECRECOVER | ✅ k256 | ✅ k256 | ✅ k256 |
| Keccak-256 | ✅ tiny-keccak | ✅ tiny-keccak | ❌ unpatched |
| BN254 ECADD | ✅ substrate-bn | ❌ ark_bn254 | ❌ ark_bn254 |
| BN254 ECMUL | ✅ substrate-bn | ✅ substrate-bn | ❌ ark_bn254 |
| BLS12-381 | ✅ sp1_bls12_381 | ❌ unpatched | ❌ unpatched |
| MODEXP | ✅ ziskos::modexp | ❌ malachite | ❌ malachite |

## Optimization Workflow

Always operate from the repository root (`cd /path/to/ethrex`).

### Phase 1: Establish Baseline

1. **Generate input for a representative block:**
   ```bash
   make -C scripts/zkvm-bench input BLOCK=23769082 RPC_URL=$RPC_URL
   ```

2. **Build and profile:**
   ```bash
   # ZisK baseline
   make -C scripts/zkvm-bench bench ZKVM=zisk BLOCK=23769082 TITLE="baseline"
   ```

3. **Convert to JSON for analysis (optional, automated by tools usually):**
   ```bash
   make -C scripts/zkvm-bench to-json FILE=scripts/zkvm-bench/profiles/zisk/stats_*_baseline.txt
   ```

### Phase 2: Analyze Bottlenecks

Examine the profile output (`stats_*.txt`) and identify targets:

#### High-Priority Targets
1. **Unpatched crypto operations** — Look for patterns like:
   - `tiny_keccak::Keccak::update` (should be `syscall_keccak_f`)
   - `ark_bn254::` (should be `substrate-bn`)
   - `sha2::sha256::compress` (should be precompile)
2. **Memory operations** — High cost in `compiler_builtins::mem::memcpy`.
3. **Serialization overhead** — `rkyv::` or `rlp::` functions.

### Phase 3: Implement Optimization

Implement **ONE** optimization at a time.

**Option A: Add/Fix Patch**
- Check `crates/l2/prover/src/guest_program/src/{zisk,sp1}/Cargo.toml`.
- Verify patch is in `[patch.crates-io]` and version matches.

**Option B: Code Optimization**
- Locate hot functions (`rg "function_name" crates/`).
- Optimizations: Replace `.clone()` with references, use `&[u8]`, avoid allocations in hot paths.
- **Constraint**: Do not optimize cold paths or break existing functionality.

### Phase 4: Validate

1. **Rebuild:**
   ```bash
   make -C scripts/zkvm-bench build ZKVM=zisk
   ```

2. **Profile (with description):**
   ```bash
   make -C scripts/zkvm-bench profile ZKVM=zisk BLOCK=23769082 TITLE="optimization_name"
   ```

3. **Compare with baseline:**
   ```bash
   make -C scripts/zkvm-bench compare \
     BASELINE=scripts/zkvm-bench/profiles/zisk/stats_*_baseline.txt \
     CURRENT=scripts/zkvm-bench/profiles/zisk/stats_*_optimization_name.txt
   ```

4. **Evaluate:**
   - **Improvement**: Keep change, update baseline, commit each optimization with a detailed message.
   - **Regression**: Revert.

### Phase 5: Report

1. Update `scripts/zkvm-bench/logbook.md`, even if the optimization failed to yield a performance gain (explain why in this case).
2. Create a report in `scripts/zkvm-bench/reports/` if deep analysis is needed.

## Decision Framework

| Symptom | Action |
|---------|--------|
| Crypto function in top 10 | Check if patch exists, add/fix it |
| too many `memcpy` | Look for unnecesary memory allocation in hot paths |
| too many serialization calls | Consider lazy deserialization or caching |
| single function too expensive | Deep dive into that function's implementation |

**Priority:** Patches > Algorithm Changes > Memory Layout > Micro-optimizations.

## Tools Reference

### Makefile (Recommended)

```bash
# Full workflow
make -C scripts/zkvm-bench bench ZKVM=zisk BLOCK=23769082 RPC_URL=$RPC TITLE="desc"

# Individual steps
make -C scripts/zkvm-bench input BLOCK=...
make -C scripts/zkvm-bench build ZKVM=zisk
make -C scripts/zkvm-bench profile ZKVM=zisk BLOCK=... TITLE="desc"
make -C scripts/zkvm-bench compare BASELINE=... CURRENT=...
```

### Direct Script Usage

Scripts are in `scripts/zkvm-bench/bin/`.

```bash
# Profile ZisK
bin/profile-zisk.sh <input_file> [output_dir] [top_roi] [description] [elf_path]

# Profile SP1
bin/profile-sp1.sh <input_file> [output_dir] [sample_rate] [description]
```

## Input Caveats

1. **Block data availability is time-limited** — Non-archival nodes prune historical block data after a short window. Once pruned, you can no longer generate input for that block via RPC. Generate inputs promptly after identifying target blocks, or use an archival node.

2. **Witness structure changes invalidate inputs** — Structural changes to the `ExecutionWitness` type (field additions, removals, type changes) will break deserialization of previously generated inputs. When switching branches or commits with witness changes, regenerate inputs.

3. **Filenames include commit hash** — Both inputs and profiles include the git commit hash in their filenames (e.g., `ethrex_mainnet_23769082_a1b2c3d_input.bin`, `stats_20250121_143022_a1b2c3d_baseline.txt`). This helps track which code version generated each file and makes it easier to identify stale inputs after witness changes.

## Safety Rules

1. **Always create a git branch** before making changes.
2. **One optimization per commit** for easy bisection.
3. **Never skip validation** — always compare before/after.
