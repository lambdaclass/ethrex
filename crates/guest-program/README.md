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

## Reproducible Builds

ethrex guest programs support reproducible builds using [ere-compiler](https://github.com/eth-act/ere) Docker images. Reproducible builds ensure that anyone can independently verify the ELF binary used in proofs.

### Why Reproducible Builds Matter

- **Verifiability**: Anyone can build the same ELF and verify it matches what's used in production
- **Trust**: No need to trust pre-built binaries; build from source and compare hashes
- **Auditing**: Security auditors can verify the exact code being proven

### Artifact Naming Convention

Release artifacts follow the naming convention:
```
<EL_NAME>-<EL_VERSION>-<ZKVM_NAME>-<ZKVM_SDK_VERSION>
```

Examples:
- `ethrex-v9_0_0-sp1-v5_0_8` (ere Program format)
- `ethrex-v9_0_0-sp1-v5_0_8.elf` (raw ELF)
- `ethrex-v9_0_0-risc0-v3_0_3` (ere Program format)
- `ethrex-v9_0_0-zisk-v0_15_0.elf` (raw ELF)

### Verifying Signatures

All release artifacts are signed with [minisign](https://jedisct1.github.io/minisign/). To verify:

```bash
# Install minisign
brew install minisign   # macOS
sudo apt install minisign  # Linux

# Download artifact and signature
wget https://github.com/lambdaclass/ethrex/releases/download/vX.Y.Z/ethrex-guests.tar.gz
tar -xzf ethrex-guests.tar.gz

# Verify signature
minisign -Vm ere/ethrex-v9_0_0-sp1-v5_0_8.elf -p minisign.pub
# Output: Signature and comment signature verified
```

### Verifying Reproducibility

To verify that a release artifact is reproducible:

```bash
# Run the verification script (requires Docker)
./scripts/verify-reproducibility.sh zisk latest

# Or specify a specific ere-compiler version
./scripts/verify-reproducibility.sh sp1 0.2.0-abcd123
```

The script builds the guest program twice and compares SHA256 hashes.

### Building Reproducible ELFs Locally

```bash
# Pull ere-compiler image
docker pull ghcr.io/eth-act/ere/ere-compiler-zisk:latest

# Build guest program
docker run --rm \
  -v $(pwd):/workspace:ro \
  -v $(pwd)/output:/output \
  ghcr.io/eth-act/ere/ere-compiler-zisk:latest \
  --compiler-kind rust-customized \
  --guest-path /workspace/crates/guest-program/bin/zisk \
  --output-path /output/ethrex-zisk

# Extract raw ELF from ere Program format
python3 scripts/extract-elf/extract-elf.py output/ethrex-zisk output/ethrex-zisk.elf
```

### ere Program Format

The ere-compiler outputs a serialized `Program` struct (bincode format) that contains:
- Raw ELF bytes
- Optional metadata

The raw ELF can be extracted using the provided script at `scripts/extract-elf/extract-elf.py`.

### Troubleshooting

**Docker permission denied**
```bash
# Add your user to the docker group
sudo usermod -aG docker $USER
# Log out and back in
```

**Build hash mismatch**
- Ensure you're using the same ere-compiler version
- Check that the source code matches the release tag
- Verify Docker images are pulled fresh: `docker pull ghcr.io/eth-act/ere/ere-compiler-<zkvm>:<version>`

## References

- [SP1 Documentation](https://docs.succinct.xyz/docs/sp1/introduction)
- [RISC Zero Documentation](https://dev.risczero.com/api)
- [ZisK Documentation](https://0xpolygonhermez.github.io/zisk/)
- [OpenVM Documentation](https://book.openvm.dev/)
- [ere (Ethereum Reproducible Execution)](https://github.com/eth-act/ere)
- [minisign](https://jedisct1.github.io/minisign/)
