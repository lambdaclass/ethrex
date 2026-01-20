# zkVM Block Execution Benchmarking Workflow Plan

## Overview

This plan describes a quick-iteration benchmarking flow for the ethrex zkVM block execution guest program, focusing on SP1 and ZisK backends. The goal is to enable rapid: develop optimization -> test execution -> profile/benchmark -> repeat cycles.

> **Full zkVM context:** See `zkvm_landscape.md` for comprehensive patch registry, installation guides, and deep analysis.

## Background: zkVM Optimization Principles

Unlike host machine optimization (minimize wall-clock time), zkVM optimization focuses on:

1. **Minimize zkVM cycles** - Proving time scales directly with cycle count
2. **Leverage precompiles** - Specialized circuits for expensive operations:
   - Keccak (67k calls = 15.9% cost in sample run)
   - Secp256k1 (add/dbl operations)
   - BLS12-381 operations
   - SHA256
3. **Reduce memory operations** - memcpy is expensive (18.6% cost in sample)
4. **Avoid unnecessary allocations** - Cloning, RLP encoding overhead
5. **No branch prediction penalty** - Control flow is cheaper than on CPUs

Current hotspots (from ZisK analysis):
- `compiler_builtins::mem::memcpy`: 18.61% of cost
- `syscall_keccak_f`: 15.90% of cost
- `tiny_keccak::Keccak::update`: 9.84% of cost
- `sp1_bls12_381::fp::Fp::sum_of_products_cpu`: 4.00% of cost

---

## Patch Utilization by Backend

Patches replace standard crypto crates with zkVM-optimized versions that call precompiles. This is the single most impactful optimization for crypto-heavy workloads.

### Summary

| Backend | Patch Coverage | Notes |
|---------|----------------|-------|
| **ZisK** | 100% (7/7) | Most optimized, has MODEXP precompile |
| **SP1** | ~80% (7-8/9) | ECADD bug, no c-kzg patch |
| **RISC0** | ~90% active | Keccak/BLS12-381 patches disabled (require "unstable") |

### Crypto Operation → Library Matrix

| Operation | Native | SP1 | RISC0 | ZisK |
|-----------|--------|-----|-------|------|
| **ECRECOVER** | `secp256k1` | `k256` ✅ | `k256` ✅ | `k256` ✅ |
| **SHA2-256** | `sha2` | `sha2` ✅ | `sha2` ✅ | `sha2` ✅ |
| **Keccak-256** | ASM | `tiny-keccak` ✅ | `tiny-keccak` ❌ | `tiny-keccak` ✅ |
| **SHA3 (EIP-7702)** | `sha3` | `sha3` ✅ | `sha3` ❌ | `sha3` ✅ |
| **BN254 ECADD** | `ark_bn254` | `ark_bn254` ❌ | `ark_bn254` ❌ | `substrate_bn` ✅ |
| **BN254 ECMUL** | `ark_bn254` | `substrate_bn` ✅ | `ark_bn254` ❌ | `substrate_bn` ✅ |
| **BN254 PAIRING** | `ark_bn254` | `substrate_bn` ✅ | `substrate_bn` ✅ | `substrate_bn` ✅ |
| **BLS12-381** | `bls12_381` (λ) | `bls12_381` (λ) ❌ | `bls12_381` (λ) ❌ | `sp1_bls12_381` ✅ |
| **KZG** | `c-kzg`/`kzg-rs` | `kzg-rs` ❌ | `c-kzg` ✅ | `kzg-rs` ✅ |
| **P256VERIFY** | `p256` | `p256` ✅ | `p256` ✅ | `p256` ❌ |
| **MODEXP** | `malachite` | `malachite` ❌ | `malachite` ❌ | `ziskos::modexp` ✅ |

Legend: ✅ = patched/accelerated, ❌ = native/unpatched, (λ) = lambdaclass fork

### Known Optimization Gaps

| Backend | Gap | Impact | Notes |
|---------|-----|--------|-------|
| **SP1** | ECADD uses `ark_bn254` | High | Bug in substrate-bn causes GasMismatch on mainnet (see `precompiles.rs:814`) |
| **SP1** | No c-kzg patch | Moderate | Uses unpatched `kzg-rs` |
| **SP1** | BLS12-381 unpatched | High | Uses lambdaclass fork |
| **RISC0** | Keccak unpatched | **Very High** | Patch requires "unstable" feature |
| **RISC0** | SHA3 unpatched | **Very High** | Patch requires "unstable" feature |
| **RISC0** | BLS12-381 unpatched | High | Patch requires "unstable" feature |
| **RISC0** | ECADD/ECMUL unpatched | High | Uses `ark_bn254` |
| **ZisK** | P256 unpatched | High | No ZisK patch exists for P256VERIFY |

### Feature Flag Propagation

```
Guest Cargo.toml → ethrex-vm → ethrex-levm → #[cfg(feature = "...")]
                            → ethrex-common
```

Features: `zisk`, `sp1`, `risc0`, `openvm`, `secp256k1`, `c-kzg`, `kzg-rs`

## Implementation Plan

### 1. Create Benchmarking Scripts Directory

Create `scripts/zkvm-bench/` with the following structure:

```
scripts/zkvm-bench/
├── build.sh           # Quick recompilation
├── profile-sp1.sh     # SP1 profiling
├── profile-zisk.sh    # ZisK profiling
├── run-bench.sh       # Run full benchmark
├── compare.py         # Compare results
└── README.md          # Documentation
```

### 2. Quick Recompilation Script (`build.sh`)

```bash
#!/bin/bash
# scripts/zkvm-bench/build.sh
# Quick rebuild for SP1 or ZisK guest programs

set -e

ZKVM=${1:-sp1}  # Default to SP1
GUEST_DIR="crates/l2/prover/src/guest_program"

case $ZKVM in
  sp1)
    echo "Building SP1 guest program..."
    cd "$GUEST_DIR/src/sp1"
    # Use incremental builds when possible
    cargo build --release
    ;;
  zisk)
    echo "Building ZisK guest program..."
    cd "$GUEST_DIR/src/zisk"
    cargo-zisk build --release
    # Output: target/riscv64ima-zisk-zkvm-elf/release/zkvm-zisk-program
    ;;
  both)
    $0 sp1
    $0 zisk
    ;;
  *)
    echo "Usage: $0 [sp1|zisk|both]"
    exit 1
    ;;
esac

echo "Build complete for $ZKVM"
```

### 3. SP1 Profiling Script (`profile-sp1.sh`)

Based on [SP1 profiling documentation](https://docs.succinct.xyz):

```bash
#!/bin/bash
# scripts/zkvm-bench/profile-sp1.sh
# Generate SP1 flamegraph profile

set -e

INPUT_FILE=${1:-"test_input.bin"}
OUTPUT_DIR=${2:-"profiles/sp1"}
SAMPLE_RATE=${3:-100}  # Higher = smaller file, less detail

mkdir -p "$OUTPUT_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
TRACE_FILE="$OUTPUT_DIR/trace_$TIMESTAMP.json"

echo "Profiling SP1 execution..."
echo "Input: $INPUT_FILE"
echo "Output: $TRACE_FILE"
echo "Sample rate: 1 in every $SAMPLE_RATE cycles"

# Build with profiling feature if not already
cd crates/l2/prover
cargo build --release --features "l2,l2-sql,sp1,profiling"

# Execute with tracing enabled
TRACE_FILE="$TRACE_FILE" \
TRACE_SAMPLE_RATE="$SAMPLE_RATE" \
cargo run --release --features "l2,l2-sql,sp1,profiling" -- \
  execute --input "$INPUT_FILE"

echo ""
echo "Profile saved to: $TRACE_FILE"
echo ""
echo "To view the profile, run:"
echo "  samply load $TRACE_FILE"
echo ""
echo "Or open http://127.0.0.1:8000/ui/flamegraph after running samply"
```

### 4. ZisK Profiling Script (`profile-zisk.sh`)

Based on `ziskemu` CLI (verified via `ziskemu --help`):

**Key flags:**
- `-e, --elf` - ELF file path
- `-i, --inputs` - Input data file path
- `-X, --stats` - Generate statistics about opcodes and memory usage
- `-S, --read-symbols` - Load function names from ELF
- `-D, --top-roi-detail` - Show detailed analysis for top functions
- `-T, --top-roi <N>` - Number of top functions to display (default: 25)
- `-C, --roi-callers <N>` - Number of callers to show per function (default: 10)

```bash
#!/bin/bash
# scripts/zkvm-bench/profile-zisk.sh
# Generate ZisK execution statistics

set -e

INPUT_FILE=${1:-"test_input.bin"}
OUTPUT_DIR=${2:-"profiles/zisk"}
TOP_ROI=${3:-25}  # Number of top functions to show
ELF_PATH="crates/l2/prover/src/guest_program/src/zisk/target/riscv64ima-zisk-zkvm-elf/release/zkvm-zisk-program"

mkdir -p "$OUTPUT_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
STATS_FILE="$OUTPUT_DIR/stats_$TIMESTAMP.txt"

echo "Profiling ZisK execution..."
echo "Input: $INPUT_FILE"
echo "ELF: $ELF_PATH"
echo "Output: $STATS_FILE"

# Run ziskemu with full statistics
# -X: Generate opcode/memory statistics
# -S: Load symbols from ELF for function names
# -D: Show detailed analysis per function
# -T: Number of top functions to display
ziskemu \
  -e "$ELF_PATH" \
  -i "$INPUT_FILE" \
  -X \
  -S \
  -D \
  -T "$TOP_ROI" \
  2>&1 | tee "$STATS_FILE"

echo ""
echo "Statistics saved to: $STATS_FILE"
echo ""

# Quick summary for terminal
echo "=== Quick Summary ==="
grep -E "^STEPS|^COST DISTRIBUTION|^TOP COST FUNCTIONS" -A 12 "$STATS_FILE" 2>/dev/null | head -30 || true
```

### 5. Benchmark Runner Script (`run-bench.sh`)

This script integrates with [ethrex-replay](https://github.com/lambdaclass/ethrex-replay), which handles:
- Fetching block data from RPC
- Generating execution witnesses
- Serializing inputs for guest programs
- Running execution/proving

```bash
#!/bin/bash
# scripts/zkvm-bench/run-bench.sh
# Run benchmarks on mainnet blocks using ethrex-replay

set -e

ZKVM=${1:-zisk}  # Default to ZisK (most performant)
BLOCK_NUM=${2:-23769082}
RPC_URL=${3:-"http://localhost:8545"}  # Use debug_executionWitness-supporting RPC
ACTION=${4:-execute}  # execute or prove
ETHREX_REPLAY_PATH=${ETHREX_REPLAY_PATH:-"../ethrex-replay"}
OUTPUT_DIR="benchmarks/$ZKVM/$(date +%Y%m%d)"

mkdir -p "$OUTPUT_DIR"

echo "Running $ZKVM $ACTION on block $BLOCK_NUM"
echo "RPC: $RPC_URL"
echo "Output: $OUTPUT_DIR"
echo ""

if [ ! -d "$ETHREX_REPLAY_PATH" ]; then
  echo "ethrex-replay not found at $ETHREX_REPLAY_PATH"
  echo "Clone it from: https://github.com/lambdaclass/ethrex-replay"
  echo ""
  echo "Or set ETHREX_REPLAY_PATH environment variable"
  exit 1
fi

cd "$ETHREX_REPLAY_PATH"

# Build features based on zkVM
case $ZKVM in
  sp1)
    FEATURES="sp1"
    ;;
  zisk)
    FEATURES="zisk"
    ;;
  *)
    echo "Unknown zkVM: $ZKVM (use sp1 or zisk)"
    exit 1
    ;;
esac

# Run ethrex-replay
cargo run -r -F "$FEATURES" -p ethrex-replay -- \
  blocks \
  --action "$ACTION" \
  --zkvm "$ZKVM" \
  --from "$BLOCK_NUM" \
  --to "$BLOCK_NUM" \
  --rpc-url "$RPC_URL" \
  2>&1 | tee "$OUTPUT_DIR/block_${BLOCK_NUM}_${ACTION}.log"

echo ""
echo "Log saved to: $OUTPUT_DIR/block_${BLOCK_NUM}_${ACTION}.log"
```

### 6. Comparison Script (`compare.py`)

```python
#!/usr/bin/env python3
"""
scripts/zkvm-bench/compare.py
Compare benchmark results between runs
"""

import sys
import json
import re
from pathlib import Path
from dataclasses import dataclass
from typing import Optional

@dataclass
class ZiskStats:
    steps: int
    total_cost: int
    top_functions: dict[str, int]

    @classmethod
    def from_file(cls, path: Path) -> 'ZiskStats':
        content = path.read_text()

        # Parse STEPS
        steps_match = re.search(r'STEPS\s+([\d,]+)', content)
        steps = int(steps_match.group(1).replace(',', '')) if steps_match else 0

        # Parse top cost functions
        top_functions = {}
        func_pattern = r'^\s*([\d,]+)\s+([\d.]+)%\s+(.+)$'
        in_top_cost = False
        for line in content.split('\n'):
            if 'TOP COST FUNCTIONS' in line:
                in_top_cost = True
                continue
            if in_top_cost:
                match = re.match(func_pattern, line.strip())
                if match:
                    cost = int(match.group(1).replace(',', ''))
                    func_name = match.group(3).strip()
                    top_functions[func_name] = cost
                elif line.strip() and not line.startswith('-'):
                    break

        total_cost = sum(top_functions.values())
        return cls(steps=steps, total_cost=total_cost, top_functions=top_functions)

def compare(baseline: Path, current: Path):
    base = ZiskStats.from_file(baseline)
    curr = ZiskStats.from_file(current)

    print(f"{'Metric':<40} {'Baseline':>15} {'Current':>15} {'Change':>12}")
    print("-" * 85)

    # Steps comparison
    step_change = ((curr.steps - base.steps) / base.steps) * 100
    print(f"{'Total Steps':<40} {base.steps:>15,} {curr.steps:>15,} {step_change:>+11.2f}%")

    # Top functions comparison
    print(f"\n{'Top Functions by Cost':^85}")
    print("-" * 85)

    all_funcs = set(base.top_functions.keys()) | set(curr.top_functions.keys())
    for func in sorted(all_funcs, key=lambda f: curr.top_functions.get(f, 0), reverse=True)[:10]:
        base_cost = base.top_functions.get(func, 0)
        curr_cost = curr.top_functions.get(func, 0)
        if base_cost > 0:
            change = ((curr_cost - base_cost) / base_cost) * 100
            change_str = f"{change:>+11.2f}%"
        else:
            change_str = "NEW"

        # Truncate function name for display
        display_name = func[:38] + ".." if len(func) > 40 else func
        print(f"{display_name:<40} {base_cost:>15,} {curr_cost:>15,} {change_str:>12}")

if __name__ == "__main__":
    if len(sys.argv) != 3:
        print("Usage: compare.py <baseline.txt> <current.txt>")
        sys.exit(1)

    compare(Path(sys.argv[1]), Path(sys.argv[2]))
```

### 7. Add Makefile Targets

Add to `crates/l2/Makefile`:

```makefile
# ==============================================================================
# Benchmarking
# ==============================================================================

bench-build-sp1: ## Build SP1 guest for benchmarking
	cd prover/src/guest_program/src/sp1 && cargo build --release

bench-build-zisk: ## Build ZisK guest for benchmarking
	cd prover/src/guest_program/src/zisk && cargo-zisk build --release

bench-profile-sp1: ## Profile SP1 execution (requires INPUT_FILE)
	TRACE_FILE=sp1_trace.json TRACE_SAMPLE_RATE=100 \
	cargo run --release --features "l2,l2-sql,sp1,profiling" -- \
	l2 bench --input $(INPUT_FILE) --backend sp1

bench-profile-zisk: ## Profile ZisK execution (requires INPUT_FILE)
	ziskemu -e prover/src/guest_program/src/zisk/out/riscv64ima-zisk-elf \
	-i $(INPUT_FILE) -X -S -D 2>&1 | tee zisk_stats.txt

bench-view-sp1: ## View SP1 profile with samply
	samply load sp1_trace.json
```

### 8. Visualization for AI Agents

Create machine-readable output format:

```bash
#!/bin/bash
# scripts/zkvm-bench/to-json.sh
# Convert profiling output to JSON for AI analysis

INPUT=$1
OUTPUT=${2:-"${INPUT%.txt}.json"}

if [[ "$INPUT" == *"zisk"* ]]; then
  # Parse ZisK stats to JSON
  python3 -c "
import re
import json
import sys

content = open('$INPUT').read()

data = {
    'type': 'zisk',
    'steps': 0,
    'cost_distribution': {},
    'top_functions': [],
    'opcodes': []
}

# Parse steps
m = re.search(r'STEPS\s+([\d,]+)', content)
if m: data['steps'] = int(m.group(1).replace(',', ''))

# Parse cost distribution
for m in re.finditer(r'^(\w+)\s+([\d,]+)\s+([\d.]+)%', content, re.M):
    if m.group(1) in ['BASE', 'MAIN', 'OPCODES', 'PRECOMPILES', 'MEMORY']:
        data['cost_distribution'][m.group(1).lower()] = {
            'cost': int(m.group(2).replace(',', '')),
            'percent': float(m.group(3))
        }

# Parse top functions
in_funcs = False
for line in content.split('\n'):
    if 'TOP COST FUNCTIONS' in line:
        in_funcs = True
        continue
    if in_funcs and line.strip():
        m = re.match(r'\s*([\d,]+)\s+([\d.]+)%\s+(.+)', line)
        if m:
            data['top_functions'].append({
                'name': m.group(3).strip(),
                'cost': int(m.group(1).replace(',', '')),
                'percent': float(m.group(2))
            })
        elif len(data['top_functions']) > 0:
            break

print(json.dumps(data, indent=2))
" > "$OUTPUT"
else
  echo "SP1 trace files are already in JSON format"
  cp "$INPUT" "$OUTPUT"
fi

echo "JSON output: $OUTPUT"
```

## Workflow Summary

### Quick Iteration Loop

```
┌─────────────────────────────────────────────────────────────┐
│                    OPTIMIZATION CYCLE                       │
└─────────────────────────────────────────────────────────────┘
        │
        ▼
┌───────────────┐    ┌───────────────┐    ┌───────────────┐
│ 1. Modify     │───▶│ 2. Build      │───▶│ 3. Profile    │
│    Code       │    │    Guest      │    │               │
│               │    │               │    │ SP1: samply   │
│ - Edit Rust   │    │ ./build.sh    │    │ ZisK: ziskemu │
│ - Add precomp │    │   [sp1|zisk]  │    │               │
│ - Optimize    │    │               │    │ (executes &   │
│               │    │               │    │  profiles)    │
└───────────────┘    └───────────────┘    └───────────────┘
        ▲                                        │
        │                                        ▼
        │                               ┌───────────────┐
        │◀──────────────────────────────│ 4. Compare    │
        │     (if regression or         │               │
        │      more optimization        │ compare.py    │
        │      needed)                  │               │
        │                               └───────────────┘
```

### Command Cheatsheet

| Task | SP1 | ZisK |
|------|-----|------|
| Build guest | `./build.sh sp1` | `./build.sh zisk` |
| Profile direct | `./profile-sp1.sh input.bin` | `./profile-zisk.sh input.bin` |
| Run on block | `./run-bench.sh sp1 23769082 $RPC` | `./run-bench.sh zisk 23769082 $RPC` |
| View profile | `samply load trace.json` | `less zisk_stats.txt` |
| Compare | `python compare.py baseline.txt current.txt` | Same |
| Convert to JSON | `./to-json.sh zisk_stats.txt` | `./to-json.sh zisk_stats.txt` |
| Full prove (GPU) | `./run-bench.sh sp1 BLOCK RPC prove` | `./run-bench.sh zisk BLOCK RPC prove` |

### Direct ziskemu Command (for manual profiling)

```bash
# Full profiling with function symbols and caller details
ziskemu -e <ELF_PATH> -i <INPUT_PATH> -X -S -D

# Quick stats only (no symbol resolution)
ziskemu -e <ELF_PATH> -i <INPUT_PATH> -X

# With custom number of top functions (default: 25)
ziskemu -e <ELF_PATH> -i <INPUT_PATH> -X -S -D -T 50
```

### Finding Missing Patches with ziskemu

When profiling shows high cycle counts in crypto functions, patches may be missing or not applied correctly.

**Diagnostic approach:**

1. Run `ziskemu -e <ELF> -i <INPUT> -D -X -S`
2. Look for these warning signs in TOP COST FUNCTIONS:
   - High cycles in `sha256`, `keccak`, `secp256k1` → missing crypto patches
   - Functions that should use precompiles but are running native code
   - Unexpected crypto library names (e.g., `ark_bn254` instead of `substrate_bn`)

**Example: Identifying unpatched operations**

```
TOP COST FUNCTIONS:
  1,234,567  15.2%  tiny_keccak::Keccak::update    ← Should be syscall_keccak_f
    987,654  12.1%  ark_bn254::g1::G1Projective    ← Should use substrate_bn
    456,789   5.6%  sha2::sha256::compress256      ← Missing sha2 patch
```

**Fix checklist:**
1. Verify patch is declared in guest `Cargo.toml` `[patch.crates-io]`
2. Check tag exists: `gh api repos/<org>/<repo>/tags`
3. Verify crate version matches patch version
4. Check zkVM SDK version compatibility
5. Look for `[patch."https://..."]` for non-crates.io sources

**Performance hierarchy** (fastest to slowest):
1. Precompile (patched crate) — 100-1000x faster
2. Native optimized (SIMD, asm)
3. Pure Rust implementation

## Files to Create

1. `scripts/zkvm-bench/build.sh` - Quick guest program rebuilds
2. `scripts/zkvm-bench/profile-sp1.sh` - SP1 flamegraph generation
3. `scripts/zkvm-bench/profile-zisk.sh` - ZisK statistics generation
4. `scripts/zkvm-bench/run-bench.sh` - Block benchmark runner
5. `scripts/zkvm-bench/compare.py` - Results comparison tool
6. `scripts/zkvm-bench/to-json.sh` - Convert to machine-readable format
7. `scripts/zkvm-bench/README.md` - Usage documentation

## Verification

After implementation, verify the workflow by:

1. Running `./build.sh sp1` and `./build.sh zisk` successfully
2. Generating a profile with `./profile-sp1.sh test_input.bin`
3. Viewing the profile with `samply load profiles/sp1/trace_*.json`
4. Running ZisK stats with `./profile-zisk.sh test_input.bin`
5. Comparing results with `python compare.py baseline.txt current.txt`

## Dependencies

- `samply` - Install with `cargo install --locked samply`
- `ziskemu` - Part of ZisK toolchain
- `cargo-zisk` - Part of ZisK toolchain
- Python 3.8+ for comparison scripts
- ethrex-replay (optional, for mainnet block benchmarks)

## Notes

### General
- SP1 profiling uses the Firefox Profiler visualization
- ZisK provides detailed opcode-level cost breakdown
- For final proving benchmarks (with GPU), use a dedicated server
- The `profiling` feature must be enabled in `sp1-sdk` for SP1 tracing

### Backend-Specific Notes

**ZisK:**
- Most optimized backend (100% patch coverage)
- Only backend with MODEXP precompile
- Use `cargo-zisk` for all build/prove operations
- Rom setup required once per ELF: `cargo-zisk rom-setup -e <ELF> -k ~/.zisk/provingKey`

**SP1:**
- ECADD has known bug with substrate-bn (causes GasMismatch on mainnet)
- BLS12-381 operations use unpatched lambdaclass fork
- Good tooling for flamegraph profiling

**RISC0:**
- Keccak/SHA3/BLS12-381 patches require "unstable" feature (not production-ready)
- ECADD/ECMUL use unpatched ark_bn254
- c-kzg patch available and working

### Input Generation

For generating block execution inputs, use [ethrex-replay](https://github.com/lambdaclass/ethrex-replay):

```bash
# Single block
ethrex-replay generate-input --block <BLOCK_NUMBER> --rpc-url <RPC_URL> --output-dir <DIR>

# Range of blocks
ethrex-replay generate-input --from <START> --to <END> --rpc-url <RPC_URL> --output-dir <DIR>

# Multiple specific blocks
ethrex-replay generate-input --blocks <B1>,<B2>,<B3> --rpc-url <RPC_URL>
```

Output format: `ethrex_<network>_<block>_input.bin` (rkyv serialized)

### Documentation Links

| Resource | URL |
|----------|-----|
| ZisK Docs | https://0xpolygonhermez.github.io/zisk/ |
| SP1 Docs | https://docs.succinct.xyz/docs/sp1/introduction |
| RISC0 Docs | https://dev.risczero.com/api |
| ethrex-replay | https://github.com/lambdaclass/ethrex-replay |
| Full patch registry | See `zkvm_landscape.md` |

