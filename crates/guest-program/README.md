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
│   ├── crypto/         # zkVM-specific Crypto implementations
│   │   ├── mod.rs
│   │   ├── shared.rs   # Shared k256/substrate-bn helpers
│   │   ├── sp1.rs
│   │   ├── risc0.rs
│   │   ├── zisk.rs
│   │   └── openvm.rs
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
cargo check -r -p ethrex-guest-program --features sp1-build-elf

# Build RISC0 guest (requires rzup toolchain or Docker)
cargo check -r -p ethrex-guest-program --features risc0-build-elf

# Build ZisK guest (requires cargo-zisk toolchain or Docker)
cargo check -r -p ethrex-guest-program --features zisk-build-elf

# Build OpenVM guest (requires cargo-openvm toolchain or Docker)
cargo check -r -p ethrex-guest-program --features openvm-build-elf

# Reproducible build using Docker
PROVER_REPRODUCIBLE_BUILD=true cargo check -r -p ethrex-guest-program --features sp1-build-elf

# Build with L2 support
cargo check -r -p ethrex-guest-program --features sp1-build-elf,l2
```

### Check Without Building ELFs

```bash
# Default (no zkVM)
cargo check -p ethrex-guest-program

# With a zkVM crypto module (no ELF build)
cargo check -p ethrex-guest-program --features sp1
```

## Features

| Feature | Description |
|---------|-------------|
| `sp1` | Base SP1 feature: enables SP1 crypto module and feature propagation |
| `risc0` | Base RISC Zero feature: enables RISC0 crypto module and feature propagation |
| `zisk` | Base ZisK feature: enables ZisK crypto module and feature propagation |
| `openvm` | Base OpenVM feature: enables OpenVM crypto module and feature propagation |
| `sp1-build-elf` | SP1 base + build tooling. Triggers `build.rs` to compile the SP1 guest ELF |
| `risc0-build-elf` | RISC Zero base + build tooling. Triggers `build.rs` to compile the RISC0 guest ELF |
| `zisk-build-elf` | ZisK base + build tooling. Triggers `build.rs` to compile the ZisK guest ELF |
| `openvm-build-elf` | OpenVM base + build tooling. Triggers `build.rs` to compile the OpenVM guest ELF |
| `l2` | Enables L2 (rollup) program mode. Used for L2 provers. Can be combined with one zkVM feature |
| `sp1-cycles` | Reports cycle counts (SP1 only) |
| `c-kzg` | Enables KZG precompile support |
| `ci` | Skip rom-setup for CI builds |

Guest binaries use base features (`sp1`, `risc0`, etc.) to get their `Crypto` implementation without triggering the build script. The prover host uses `-build-elf` features which include the base feature plus build tooling.

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

### ZisK v0.16.1 (Polygon)
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

1. A `Crypto` implementation in `src/crypto/<zkvm>.rs`
2. A guest binary crate in `bin/<zkvm>/`
3. A build function in `build.rs`
4. Feature flags in `Cargo.toml`
5. ELF constants in `src/lib.rs`
6. Makefile targets

### Step-by-Step Guide

#### 1. Implement the `Crypto` Trait

Create `src/crypto/<zkvm>.rs` with your zkVM's cryptographic implementations. The `Crypto` trait (from `ethrex-crypto`) defines all EVM cryptographic operations with default native implementations. Override the methods that your zkVM provides accelerated versions for:

```rust
use ethrex_crypto::Crypto;

pub struct MyZkvmCrypto;

impl Crypto for MyZkvmCrypto {
    // Override methods where your zkVM has optimized implementations.
    // Methods you don't override will use the default native implementations.
}
```

Most zkVMs provide accelerated `k256` (secp256k1 ECDSA) and `substrate-bn` (BN254) through `[patch.crates-io]` in the guest binary's `Cargo.toml`. See `src/crypto/shared.rs` for helper functions that use these libraries. Register your module in `src/crypto/mod.rs` with a `#[cfg(feature = "<zkvm>")]` gate.

#### 2. Create the Guest Binary Crate

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

# The main guest program library — use the BASE feature, not -build-elf
ethrex-guest-program = { path = "../../", default-features = false, features = ["<zkvm>"] }

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

use std::sync::Arc;

// Import L1 or L2 program based on feature flag
#[cfg(feature = "l2")]
use ethrex_guest_program::l2::{ProgramInput, execution_program};
#[cfg(not(feature = "l2"))]
use ethrex_guest_program::l1::{ProgramInput, execution_program};

// Import your Crypto implementation
use ethrex_guest_program::crypto::<zkvm>::MyZkvmCrypto;

use rkyv::rancor::Error;

// Use your zkVM's entrypoint macro
<zkvm>::entrypoint!(main);

pub fn main() {
    // 1. Read input bytes using your zkVM's IO
    let input = <zkvm>::io::read_vec();

    // 2. Deserialize input
    let input = rkyv::from_bytes::<ProgramInput, Error>(&input).unwrap();

    // 3. Execute the program with your crypto provider
    let crypto = Arc::new(MyZkvmCrypto);
    let output = execution_program(input, crypto).unwrap();

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

#### 3. Add Build Function in `build.rs`

Add a new build function for your zkVM. Gate it on `<zkvm>-build-elf`, not the base feature:

```rust
#[cfg(all(not(clippy), feature = "<zkvm>-build-elf"))]
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

    #[cfg(all(not(clippy), feature = "<zkvm>-build-elf"))]
    build_<zkvm>_program();
}
```

#### 4. Add Feature Flags in `Cargo.toml`

Add two features to `ethrex-guest-program/Cargo.toml`:

```toml
[build-dependencies]
# Add build-time SDK dependency if needed (optional)
<zkvm>-build = { version = "=X.Y.Z", optional = true }

[features]
# Base feature: crypto module + feature propagation. Used by guest binaries.
<zkvm> = ["dep:k256", "dep:substrate-bn"]

# Build-ELF feature: base + build tooling. Used by the prover host.
# Guest binaries must NOT enable this — it triggers build.rs which
# recompiles the guest binary.
<zkvm>-build-elf = ["<zkvm>"]
```

If your zkVM has build-time Rust dependencies (like `sp1-build` or `risc0-build`), add them to the `-build-elf` feature:

```toml
<zkvm>-build-elf = ["<zkvm>", "dep:<zkvm>-build"]
```

#### 5. Add ELF Constants in `src/lib.rs`

Add static constants for the compiled ELF, gated on `-build-elf`:

```rust
#[cfg(all(not(clippy), feature = "<zkvm>-build-elf"))]
pub static ZKVM_<ZKVM>_PROGRAM_ELF: &[u8] =
    include_bytes!("../bin/<zkvm>/out/<target-triple>-<zkvm>-elf");
#[cfg(any(clippy, not(feature = "<zkvm>-build-elf")))]
pub const ZKVM_<ZKVM>_PROGRAM_ELF: &[u8] = &[];

// If your zkVM produces a verification key:
#[cfg(all(not(clippy), feature = "<zkvm>-build-elf"))]
pub static ZKVM_<ZKVM>_PROGRAM_VK: &str =
    include_str!("../bin/<zkvm>/out/<target-triple>-<zkvm>-vk");
#[cfg(any(clippy, not(feature = "<zkvm>-build-elf")))]
pub const ZKVM_<ZKVM>_PROGRAM_VK: &str = "";
```

#### 6. Add Makefile Targets

Add to `Makefile`:

```makefile
.PHONY: <zkvm> l2-<zkvm>

<zkvm>:
	$(ENV_PREFIX) cargo check $(CARGO_FLAGS) --features <zkvm>-build-elf

l2-<zkvm>:
	$(ENV_PREFIX) cargo check $(CARGO_FLAGS) --features <zkvm>-build-elf,l2
```

Update the `clean` target:

```makefile
clean:
	rm -rf bin/sp1/out bin/risc0/out bin/zisk/out bin/openvm/out bin/<zkvm>/out
```

Update the `help` target to include your new zkVM.

#### 7. Add Patches for Cryptographic Libraries

Most zkVMs have optimized patches for cryptographic libraries. Common libraries that benefit from patches:

- `tiny-keccak` (Keccak256 for state root hashing)
- `sha2` (SHA256)
- `secp256k1` / `k256` (ECDSA signature verification)
- `p256` (P256 curve for EIP-7212)
- `substrate-bn` (BN254 for EIP-196/197 precompiles)
- `bls12_381` (BLS12-381 for EIP-2537)

Check your zkVM's documentation or patches repository for available optimizations.

#### 8. (Optional) Add CI Workflow

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
