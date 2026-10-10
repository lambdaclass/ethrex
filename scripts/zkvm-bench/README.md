# zkVM Benchmarking Scripts

Quick-iteration benchmarking tools for ethrex zkVM block execution guest programs.

## Overview

These scripts enable rapid optimization cycles:
1. **Modify code** → 2. **Build guest** → 3. **Profile** → 4. **Compare** → repeat

## Prerequisites

- **samply**: `cargo install --locked samply` (for SP1 flamegraphs)
- **ziskemu**: Part of ZisK toolchain (`cargo-zisk sdk install-toolchain`)
- **cargo-zisk**: Part of ZisK toolchain
- **Python 3.8+**: For comparison scripts
- **ethrex-replay**: For generating block inputs (https://github.com/lambdaclass/ethrex-replay)

## Quick Start (Makefile)

The easiest way to use these tools is via the Makefile:

```bash
cd scripts/zkvm-bench

# Full workflow: generate input (if needed) -> build -> profile
make bench ZKVM=zisk BLOCK=23769082 RPC_URL=$RPC_URL

# Individual steps
make input BLOCK=23769082 RPC_URL=$RPC_URL    # Generate input
make build ZKVM=zisk                           # Build guest program
make profile ZKVM=zisk BLOCK=23769082          # Profile execution
make compare BASELINE=before.txt CURRENT=after.txt  # Compare results

# Utilities
make list-inputs                               # List available inputs
make list-profiles                             # List profiles
make help                                      # Show all commands
```

## Quick Start (Scripts)

You can also use the scripts directly (located in `bin/`):

```bash
# Generate input
bin/generate-input.sh 23769082 $RPC_URL

# Build guest program
bin/build.sh zisk     # or sp1

# Profile execution
bin/profile-zisk.sh inputs/ethrex_mainnet_23769082_input.bin

# Compare results
python bin/compare.py profiles/zisk/stats_baseline.txt profiles/zisk/stats_current.txt

# Convert to JSON for tooling
bin/to-json.sh profiles/zisk/stats_latest.txt
```

## Scripts

### generate-input.sh

Generate block execution inputs using ethrex-replay.

```bash
bin/generate-input.sh <block_number> [rpc_url] [output_dir]

# Examples
bin/generate-input.sh 23769082
bin/generate-input.sh 23769082 $RPC_URL inputs/
```

Requires ethrex-replay (set `ETHREX_REPLAY_PATH` env var if not at `../ethrex-replay`).

### build.sh

Build SP1 or ZisK guest programs.

```bash
bin/build.sh sp1      # Build SP1 guest
bin/build.sh zisk     # Build ZisK guest
bin/build.sh both     # Build both
```

### profile-sp1.sh

Generate SP1 flamegraph profiles using tracing.

```bash
bin/profile-sp1.sh <input_file> [output_dir] [sample_rate] [description]

# Example
bin/profile-sp1.sh inputs/block.bin profiles/sp1 100 "baseline"
```

Output: JSON trace file viewable with `samply load <trace.json>`

### profile-zisk.sh

Generate ZisK execution statistics using ziskemu.

```bash
bin/profile-zisk.sh <input_file> [output_dir] [top_roi] [description] [elf_path]

# Example
bin/profile-zisk.sh inputs/block.bin profiles/zisk 50 "optimization_1"
```

Output: Text file with cycle counts, cost distribution, and top functions.

### run-bench.sh

Run benchmarks on mainnet blocks using ethrex-replay.

```bash
bin/run-bench.sh <zkvm> <block_number> [rpc_url] [action]

# Examples
bin/run-bench.sh zisk 23769082 http://localhost:8545 execute
bin/run-bench.sh sp1 23769082 $RPC_URL prove
```

Requires ethrex-replay repository (set `ETHREX_REPLAY_PATH` env var).

### compare.py

Compare benchmark results between runs.

```bash
python bin/compare.py <baseline.txt> <current.txt>

# Example
python bin/compare.py profiles/zisk/stats_before.txt profiles/zisk/stats_after.txt
```

Shows:
- Total steps comparison
- Cost distribution changes
- Per-function cost changes
- New/removed functions

### to-json.sh

Convert profiling output to JSON for AI analysis.

```bash
bin/to-json.sh <input_file> [output_file]

# Example
bin/to-json.sh profiles/zisk/stats.txt
```

## Workflow

### Optimization Cycle

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

**Using Makefile (recommended):**

| Task | Command |
|------|---------|
| Full workflow | `make bench ZKVM=zisk BLOCK=123 RPC_URL=$RPC` |
| Generate input | `make input BLOCK=123 RPC_URL=$RPC` |
| Build guest | `make build ZKVM=zisk` |
| Profile | `make profile ZKVM=zisk BLOCK=123` |
| Compare | `make compare BASELINE=a.txt CURRENT=b.txt` |
| List inputs | `make list-inputs` |
| List profiles | `make list-profiles` |

**Using scripts directly:**

| Task | SP1 | ZisK |
|------|-----|------|
| Generate input | `./generate-input.sh 123 $RPC` | Same |
| Build guest | `./build.sh sp1` | `./build.sh zisk` |
| Profile | `./profile-sp1.sh input.bin` | `./profile-zisk.sh input.bin` |
| View profile | `samply load trace.json` | `less stats.txt` |
| Compare | `python compare.py base.txt curr.txt` | Same |
| To JSON | `./to-json.sh stats.txt` | Same |

## Finding Missing Patches

Use ziskemu output to identify unpatched crypto operations:

```bash
./profile-zisk.sh input.bin
```

Look for these warning signs in TOP COST FUNCTIONS:
- High cycles in `sha256`, `keccak`, `secp256k1` → missing crypto patches
- `ark_bn254` instead of `substrate_bn` → BN254 not using precompile
- `tiny_keccak::Keccak::update` instead of `syscall_keccak_f` → Keccak not patched

## Input Files

Generate inputs using the provided script or Makefile:

```bash
# Using Makefile
make input BLOCK=23769082 RPC_URL=$RPC

# Using script directly
./generate-input.sh 23769082 $RPC

# Or use ethrex-replay directly for ranges
ethrex-replay generate-input --from 23769082 --to 23769090 --rpc-url $RPC --output-dir inputs/
```

Output format: `ethrex_mainnet_<block>_input.bin`

Inputs are cached in `scripts/zkvm-bench/inputs/` - existing files won't be regenerated.

## Backend Comparison

| Backend | Patch Coverage | Best For |
|---------|----------------|----------|
| **ZisK** | 100% | Production benchmarks, lowest cycle count |
| **SP1** | ~80% | Flamegraph profiling, debugging |
| **RISC0** | ~90% | When c-kzg needed |

See `zkvm_landscape.md` and `zkvm_optimization_workflow.md` for detailed patch analysis.

## Documentation

- [zkvm_optimization_workflow.md](docs/zkvm_optimization_workflow.md) - Full workflow documentation
- [zkvm_landscape.md](docs/zkvm_landscape.md) - Patch registry and zkVM details
- [ZisK Docs](https://0xpolygonhermez.github.io/zisk/)
- [SP1 Docs](https://docs.succinct.xyz/docs/sp1/introduction)
