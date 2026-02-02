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

## Integrating a New zkVM

This section describes how to add support for a new zkVM backend to ethrex.

### Overview

Each zkVM integration requires:

1. A guest binary crate in `bin/<zkvm>/`
2. A build function in `build.rs`
3. Feature flags in `Cargo.toml`
4. ELF constants in `src/lib.rs`
5. Makefile targets

### Step-by-Step Guide

#### 1. Create the Guest Binary Crate

Create a new directory `bin/<zkvm>/` with the following structure:

```
bin/<zkvm>/
├── Cargo.toml
├── Cargo.lock
└── src/
    └── main.rs
```

**Cargo.toml**:

```toml
[package]
name = "ethrex-guest-<zkvm>"
version = "9.0.0"
edition = "2024"

[workspace]

[profile.release]
lto = "thin"
codegen-units = 1

[dependencies]
# Add your zkVM SDK dependency
<zkvm>-zkvm = { version = "=X.Y.Z" }

# Required for input deserialization
rkyv = { version = "0.8.10", features = ["std", "unaligned"] }

# The main guest program library
ethrex-guest-program = { path = "../../" }

# VM with zkVM-specific features (if needed)
ethrex-vm = { path = "../../../vm", default-features = false, features = ["<zkvm>"] }

# Add patches for cryptographic libraries optimized for your zkVM
[patch.crates-io]
# Example: SHA2, Keccak, secp256k1, etc.
# tiny-keccak = { git = "https://github.com/<zkvm>-patches/tiny-keccak", tag = "..." }

[features]
l2 = ["ethrex-guest-program/l2"]
```

**src/main.rs**:

```rust
#![no_main]

// Import L1 or L2 program based on feature flag
#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(not(feature = "l2"))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};

use rkyv::rancor::Error;

// Use your zkVM's entrypoint macro
<zkvm>::entrypoint!(main);

pub fn main() {
    // 1. Read input bytes using your zkVM's IO
    let input = <zkvm>::io::read_vec();

    // 2. Deserialize input
    let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();

    // 3. Execute the program (this is the same for all zkVMs)
    let output = execution_program(input).unwrap();

    // 4. Commit output using your zkVM's commit mechanism
    //    Some zkVMs commit raw bytes, others require hashing first
    <zkvm>::io::commit(&output.encode());
}
```

The pattern for output commitment varies by zkVM:

| zkVM | Output Commitment |
|------|-------------------|
| SP1 | `sp1_zkvm::io::commit_slice(&output.encode())` |
| RISC0 | `risc0_zkvm::guest::env::commit_slice(&output.encode())` |
| ZisK | Hash with SHA256, then `ziskos::set_output(idx, u32)` for each chunk |
| OpenVM | Hash with Keccak256, then `openvm::io::reveal_bytes32(hash)` |

#### 2. Add Build Function in `build.rs`

Add a new build function for your zkVM:

```rust
#[cfg(all(not(clippy), feature = "<zkvm>"))]
fn build_<zkvm>_program() {
    use std::{fs, path::Path, process::{Command, Stdio}};

    // 1. Build the guest binary using your zkVM's toolchain
    let status = Command::new("cargo")
        .arg("<zkvm>")  // or the appropriate build command
        .arg("build")
        .arg("--release")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .current_dir("./bin/<zkvm>")
        .status()
        .expect("failed to build <zkvm> guest");

    if !status.success() {
        panic!("<zkvm> build failed");
    }

    // 2. Create output directory
    let _ = fs::create_dir("./bin/<zkvm>/out");

    // 3. Copy ELF to output directory
    fs::copy(
        "./bin/<zkvm>/target/<target-triple>/release/ethrex-guest-<zkvm>",
        "./bin/<zkvm>/out/<target-triple>-<zkvm>-elf",
    ).expect("failed to copy ELF");

    // 4. (Optional) Generate verification key if your zkVM produces one
}
```

Then call it from `main()`:

```rust
fn main() {
    // ... existing code ...

    #[cfg(all(not(clippy), feature = "<zkvm>"))]
    build_<zkvm>_program();
}
```

#### 3. Add Feature Flags in `Cargo.toml`

Add to the root `Cargo.toml`:

```toml
[build-dependencies]
# Add build-time SDK dependency if needed
<zkvm>-build = { version = "=X.Y.Z", optional = true }

[features]
<zkvm> = ["dep:<zkvm>-build"]  # Add any other required features
```

If your zkVM requires feature flags in dependent crates:

```toml
[features]
<zkvm> = [
    "dep:<zkvm>-build",
    "ethrex-common/<zkvm>",
    "ethrex-vm/<zkvm>",
    "ethrex-l2-common/<zkvm>",
]
```

#### 4. Add ELF Constants in `src/lib.rs`

Add static constants for the compiled ELF:

```rust
#[cfg(all(not(clippy), feature = "<zkvm>"))]
pub static ZKVM_<ZKVM>_PROGRAM_ELF: &[u8] =
    include_bytes!("../bin/<zkvm>/out/<target-triple>-<zkvm>-elf");
#[cfg(any(clippy, not(feature = "<zkvm>")))]
pub const ZKVM_<ZKVM>_PROGRAM_ELF: &[u8] = &[];

// If your zkVM produces a verification key:
#[cfg(all(not(clippy), feature = "<zkvm>"))]
pub static ZKVM_<ZKVM>_PROGRAM_VK: &str =
    include_str!("../bin/<zkvm>/out/<target-triple>-<zkvm>-vk");
#[cfg(any(clippy, not(feature = "<zkvm>")))]
pub const ZKVM_<ZKVM>_PROGRAM_VK: &str = "";
```

#### 5. Add Makefile Targets

Add to `Makefile`:

```makefile
.PHONY: <zkvm> l2-<zkvm>

<zkvm>:
	$(ENV_PREFIX) cargo check $(CARGO_FLAGS) --features <zkvm>

l2-<zkvm>:
	$(ENV_PREFIX) cargo check $(CARGO_FLAGS) --features <zkvm>,l2
```

Update the `clean` target:

```makefile
clean:
	rm -rf bin/sp1/out bin/risc0/out bin/zisk/out bin/openvm/out bin/<zkvm>/out
```

Update the `help` target to include your new zkVM.

#### 6. Add Patches for Cryptographic Libraries

Most zkVMs have optimized patches for cryptographic libraries. Common libraries that benefit from patches:

- `tiny-keccak` (Keccak256 for state root hashing)
- `sha2` (SHA256)
- `secp256k1` / `k256` (ECDSA signature verification)
- `p256` (P256 curve for EIP-7212)
- `substrate-bn` (BN254 for EIP-196/197 precompiles)
- `bls12_381` (BLS12-381 for EIP-2537)

Check your zkVM's documentation or patches repository for available optimizations.

#### 7. (Optional) Add CI Workflow

Add your zkVM to the CI workflow in `.github/workflows/`. See existing workflows for SP1, RISC0, ZisK, and OpenVM as examples.

### Testing Your Integration

1. **Build the guest**:
   ```bash
   cd crates/guest-program
   make <zkvm>
   ```

2. **Check ELF was created**:
   ```bash
   ls -la bin/<zkvm>/out/
   ```

3. **Run with the prover** (requires host-side integration):
   The prover backend code in `crates/l2/prover/` needs to be updated to support your zkVM for end-to-end proving.

### Architecture Notes

- **Guest programs are deterministic**: The same input must always produce the same output
- **No networking or filesystem access**: zkVMs execute in an isolated environment
- **Memory constraints**: Some zkVMs have limited heap size; optimize for memory usage
- **Cryptographic operations are expensive**: Use patched libraries when available
- **Output format varies**: Some zkVMs commit raw bytes, others require hashing

## References

- [SP1 Documentation](https://docs.succinct.xyz/docs/sp1/introduction)
- [RISC Zero Documentation](https://dev.risczero.com/api)
- [ZisK Documentation](https://0xpolygonhermez.github.io/zisk/)
- [OpenVM Documentation](https://book.openvm.dev/)
