# ethrex-guest-program

Guest program for zkVM execution in ethrex. This crate contains the code that runs inside zero-knowledge virtual machines (zkVMs) to generate proofs of Ethereum block execution.

## Architecture

```
guest-program/
├── Cargo.toml          # Main crate configuration
├── Makefile            # Build commands for each zkVM
├── build.rs            # Builds zkVM binaries for each guest
├── src/
│   ├── lib.rs          # Library exports and ELF constants
│   ├── common/         # Shared execution logic
│   │   ├── mod.rs
│   │   ├── execution.rs
│   │   └── error.rs
│   ├── l1/             # L1 (mainnet) program
│   │   ├── mod.rs
│   │   ├── input.rs
│   │   ├── output.rs
│   │   └── program.rs
│   ├── l2/             # L2 (rollup) program
│   │   ├── mod.rs
│   │   ├── input.rs
│   │   ├── output.rs
│   │   ├── program.rs
│   │   ├── blobs.rs
│   │   ├── messages.rs
│   │   └── error.rs
│   └── methods.rs      # zkVM method helpers
└── bin/                # zkVM-specific guest implementations
    ├── risc0/          # RISC Zero guest
    ├── sp1/            # Succinct SP1 guest
    ├── zisk/           # Polygon ZisK guest
    └── openvm/         # Axiom OpenVM guest
```

## Prerequisites

Building the guest program requires either:

**Option 1: Local build (default)**
- The zkVM toolchain for your target installed at the correct version (see [zkVM Guest Implementations](#zkvm-guest-implementations) for versions)

**Option 2: Reproducible build**
- Docker installed
- Set `PROVER_REPRODUCIBLE_BUILD=true` environment variable

## Building

The guest program ELF is built via `build.rs` when running cargo check/build with a zkVM feature.

### Using Make

```bash
cd crates/guest-program

# L1 guests
make sp1                    # Build SP1 guest
make risc0                  # Build RISC0 guest
make zisk                   # Build ZisK guest
make openvm                 # Build OpenVM guest

# L2 guests
make l2-sp1                 # Build SP1 guest with L2 support
make l2-risc0               # Build RISC0 guest with L2 support
make l2-zisk                # Build ZisK guest with L2 support
make l2-openvm              # Build OpenVM guest with L2 support

# Reproducible build with Docker
make sp1 REPRODUCIBLE=1
make l2-sp1 REPRODUCIBLE=1
```

### Using Cargo

```bash
# Build SP1 guest (requires sp1up toolchain or Docker)
cargo check -r -p ethrex-guest-program --features sp1

# Build RISC0 guest (requires rzup toolchain or Docker)
cargo check -r -p ethrex-guest-program --features risc0

# Build ZisK guest (requires cargo-zisk toolchain or Docker)
cargo check -r -p ethrex-guest-program --features zisk

# Build OpenVM guest (requires cargo-openvm toolchain or Docker)
cargo check -r -p ethrex-guest-program --features openvm

# Reproducible build using Docker
PROVER_REPRODUCIBLE_BUILD=true cargo check -r -p ethrex-guest-program --features sp1

# Build with L2 support
cargo check -r -p ethrex-guest-program --features sp1,l2
```

### Check Without Building ELFs

```bash
cargo check -p ethrex-guest-program
```

## Features

| Feature | Description |
|---------|-------------|
| `risc0` | Builds RISC Zero guest (mutually exclusive with other zkVM features) |
| `sp1` | Builds SP1 guest (mutually exclusive with other zkVM features) |
| `zisk` | Builds ZisK guest (mutually exclusive with other zkVM features) |
| `openvm` | Builds OpenVM guest (mutually exclusive with other zkVM features) |
| `l2` | Enables L2 (rollup) program mode. Used for L2 provers. Can be combined with one zkVM feature |
| `sp1-cycles` | Reports cycle counts (SP1 only) |
| `c-kzg` | Enables KZG precompile support |
| `ci` | Skip rom-setup for CI builds |

## zkVM Guest Implementations

Each subdirectory in `bin/` contains a guest implementation for a specific zkVM. The ethrex execution logic serves as the backend that each zkVM guest calls into.

### SP1 v5.0.8 (Succinct)
- **Architecture**: RISC-V 32-bit
- **ELF output**: `bin/sp1/out/riscv32im-succinct-zkvm-elf`
- **Patches**: Optimized crypto libraries for SHA2, SHA3, K256, etc.

### RISC0 v3.0.3
- **Architecture**: RISC-V 32-bit
- **ELF output**: `bin/risc0/out/riscv32im-risc0-elf`
- **VK output**: `bin/risc0/out/riscv32im-risc0-vk`

### ZisK v0.15.0 (Polygon)
- **Architecture**: RISC-V 64-bit
- **ELF output**: `bin/zisk/out/riscv64ima-zisk-elf`
- **Requires**: `cargo-zisk` toolchain installed

### OpenVM v1.4.1 (Axiom)
- **Architecture**: RISC-V 32-bit
- **ELF output**: `bin/openvm/out/riscv32im-openvm-elf`

## Data Flow

```
┌───────────────────────────────────────────────────────────────────────┐
│                            Host (Prover)                              │
│                                                                       │
│  ┌─────────────┐    ┌────────────────────┐    ┌────────────────┐     │
│  │ ProgramInput│───>│ ethrex-guest-program│───>│ ProgramOutput  │     │
│  │ (blocks,    │    │    (zkVM exec)      │    │ (state root,   │     │
│  │  witnesses) │    └────────────────────┘    │  receipts root)│     │
│  └─────────────┘                              └────────────────┘     │
└───────────────────────────────────────────────────────────────────────┘
```

1. **Input**: `ProgramInput` contains block data and execution witnesses
2. **Execution**: The guest program re-executes blocks inside the zkVM
3. **Output**: `ProgramOutput` contains the resulting state/receipts roots

## References

- [SP1 Documentation](https://docs.succinct.xyz/docs/sp1/introduction)
- [RISC Zero Documentation](https://dev.risczero.com/api)
- [ZisK Documentation](https://0xpolygonhermez.github.io/zisk/)
- [OpenVM Documentation](https://book.openvm.dev/)
