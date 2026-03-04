# Guest Program Developer Guide

This guide explains how to create a custom guest program for the Tokamak zkVM framework.  A guest program is a self-contained RISC-V binary that runs inside a zkVM (SP1, RISC0, ZisK, OpenVM) and produces a cryptographic proof of correct execution.

## Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│ crates/guest-program/                               │
│                                                     │
│  src/                                               │
│   ├── traits.rs          GuestProgram trait + ELF   │
│   │                      validation utilities       │
│   ├── programs/                                     │
│   │   ├── mod.rs         Module registry            │
│   │   ├── dynamic.rs     Runtime ELF loader         │
│   │   ├── evm_l2.rs      Default EVM-L2 program     │
│   │   ├── zk_dex/        ZK-DEX reference           │
│   │   │   ├── mod.rs     GuestProgram impl + tests  │
│   │   │   ├── types.rs   Input/output types (rkyv)  │
│   │   │   └── execution.rs  Business logic          │
│   │   └── tokamon/       Tokamon reference           │
│   │       ├── mod.rs                                │
│   │       ├── types.rs                              │
│   │       └── execution.rs                          │
│   └── lib.rs             ELF constants + re-exports │
│                                                     │
│  bin/                                               │
│   ├── sp1/               EVM-L2 SP1 binary          │
│   ├── sp1-zk-dex/        ZK-DEX SP1 binary          │
│   ├── sp1-tokamon/       Tokamon SP1 binary          │
│   ├── risc0/             RISC0 binary               │
│   └── zisk/              ZisK binary                │
│                                                     │
│  scripts/                                           │
│   └── new-guest-program.sh  Scaffold generator      │
└─────────────────────────────────────────────────────┘
```

### Key Concepts

- **`GuestProgram` trait** (`traits.rs`): The core abstraction.  Every guest program implements this trait.
- **Program Registry** (`crates/l2/prover/src/registry.rs`): Maps `program_id` → `Arc<dyn GuestProgram>` at prover startup.
- **ELF binary**: The compiled RISC-V executable that runs inside the zkVM.  One ELF per backend per program.
- **`program_type_id`**: An integer (u8) used on L1 to identify the program type in the VK mapping.

---

## Quick Start: Scaffold a New Program

The fastest way to create a new guest program is the scaffold script:

```bash
# From the repository root
./scripts/new-guest-program.sh my-awesome-program
```

This generates:

| File | Purpose |
|------|---------|
| `src/programs/my_awesome_program/types.rs` | Input/output types with rkyv + serde |
| `src/programs/my_awesome_program/execution.rs` | Execution logic skeleton |
| `src/programs/my_awesome_program/mod.rs` | `GuestProgram` trait impl + 8 tests |
| `bin/sp1-my-awesome-program/Cargo.toml` | SP1 guest binary config |
| `bin/sp1-my-awesome-program/src/main.rs` | SP1 zkVM entry point |

The scaffold also auto-registers the module in `programs/mod.rs` and assigns the next available `program_type_id`.

After scaffolding, customize these three files:

1. **`types.rs`** — Define your input/output data structures
2. **`execution.rs`** — Implement your business logic
3. **`mod.rs`** — Update `elf()` once you've compiled the ELF

---

## Step-by-Step: Manual Creation

### 1. Define Types (`types.rs`)

Input types **must** derive `rkyv` traits (for zkVM serialization) and `serde` traits (for JSON/config):

```rust
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, RSerialize, RDeserialize, Archive, Clone, Debug)]
pub struct MyProgramInput {
    pub initial_state_root: [u8; 32],
    // Your domain-specific fields:
    pub transfers: Vec<Transfer>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MyProgramOutput {
    pub initial_state_root: [u8; 32],
    pub final_state_root: [u8; 32],
    pub item_count: u64,
}
```

The output type needs an `encode()` method that produces the byte layout the L1 verifier expects:

```rust
impl MyProgramOutput {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(72);
        buf.extend_from_slice(&self.initial_state_root);
        buf.extend_from_slice(&self.final_state_root);
        buf.extend_from_slice(&self.item_count.to_be_bytes());
        buf
    }
}
```

### 2. Implement Execution (`execution.rs`)

The execution function is the core business logic.  It takes the input, validates it, computes a state transition, and returns the output:

```rust
use ethrex_crypto::keccak::keccak_hash;
use super::types::{MyProgramInput, MyProgramOutput};

#[derive(Debug, thiserror::Error)]
pub enum MyExecutionError {
    #[error("Empty input")]
    EmptyInput,
    #[error("Invalid transfer: {0}")]
    InvalidTransfer(String),
}

pub fn execution_program(
    input: MyProgramInput,
) -> Result<MyProgramOutput, MyExecutionError> {
    if input.transfers.is_empty() {
        return Err(MyExecutionError::EmptyInput);
    }

    let mut state = input.initial_state_root;

    for transfer in &input.transfers {
        // Validate each transfer...
        // Update state deterministically:
        let mut preimage = Vec::new();
        preimage.extend_from_slice(&state);
        // ... append transfer data ...
        state = keccak_hash(&preimage);
    }

    Ok(MyProgramOutput {
        initial_state_root: input.initial_state_root,
        final_state_root: state,
        item_count: input.transfers.len() as u64,
    })
}
```

**Important**: The execution function must be **deterministic** — same input always produces the same output.  The zkVM will re-execute this inside the prover.

### 3. Implement the Trait (`mod.rs`)

```rust
pub mod execution;
pub mod types;

use crate::traits::{GuestProgram, GuestProgramError, backends};

pub struct MyGuestProgram;

impl MyGuestProgram {
    fn non_empty(elf: &[u8]) -> Option<&[u8]> {
        if elf.is_empty() || elf == [0] { None } else { Some(elf) }
    }
}

impl GuestProgram for MyGuestProgram {
    fn program_id(&self) -> &str {
        "my-program"
    }

    fn elf(&self, backend: &str) -> Option<&[u8]> {
        match backend {
            backends::SP1 => Self::non_empty(crate::ZKVM_SP1_MY_PROGRAM_ELF),
            _ => None,
        }
    }

    fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> {
        None
    }

    fn program_type_id(&self) -> u8 {
        4 // Unique integer for L1 VK mapping
    }
}
```

### 4. Create the SP1 Binary

Each guest program needs a zkVM entry point binary.  For SP1:

**`bin/sp1-my-program/Cargo.toml`**:
```toml
[package]
name = "ethrex-guest-sp1-my-program"
version = "9.0.0"
edition = "2024"
license = "MIT OR Apache-2.0"

[workspace]

[profile.release]
lto = "thin"
codegen-units = 1

[dependencies]
sp1-zkvm = { version = "=5.0.8" }
rkyv = { version = "0.8.10", features = ["std", "unaligned"] }
ethrex-guest-program = { path = "../../", default-features = false }

[patch.crates-io]
tiny-keccak = { git = "https://github.com/sp1-patches/tiny-keccak", tag = "patch-2.0.2-sp1-4.0.0" }
```

**`bin/sp1-my-program/src/main.rs`**:
```rust
#![no_main]

use ethrex_guest_program::programs::my_program::execution::execution_program;
use ethrex_guest_program::programs::my_program::types::MyProgramInput;
use rkyv::rancor::Error;

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read_vec();
    let input = rkyv::from_bytes::<MyProgramInput, Error>(&input).unwrap();

    let output = execution_program(input).unwrap();

    sp1_zkvm::io::commit_slice(&output.encode());
}
```

**Key points**:
- `#![no_main]` and `sp1_zkvm::entrypoint!(main)` are required for SP1
- Input is read via `sp1_zkvm::io::read_vec()` (raw bytes)
- Output is committed via `sp1_zkvm::io::commit_slice()` (public values)
- The `tiny-keccak` patch is needed for `ethrex-crypto` keccak on riscv32

### 5. Wire Up the Build System

**`lib.rs`** — Add ELF constants:
```rust
#[cfg(all(not(clippy), feature = "sp1"))]
pub static ZKVM_SP1_MY_PROGRAM_ELF: &[u8] =
    include_bytes!("../bin/sp1-my-program/out/riscv32im-succinct-zkvm-elf");
#[cfg(any(clippy, not(feature = "sp1")))]
pub const ZKVM_SP1_MY_PROGRAM_ELF: &[u8] = &[];
```

**`build.rs`** — Add build function (same pattern as existing ones):
```rust
if programs.contains(&"my-program".to_string()) {
    #[cfg(all(not(clippy), feature = "sp1"))]
    build_sp1_my_program();
}
```

**`Makefile`** — Add target:
```makefile
sp1-my-program:
	$(ENV_PREFIX) GUEST_PROGRAMS=my-program cargo check $(CARGO_FLAGS) --features sp1
```

### 6. Register in the Prover

In `crates/l2/prover/src/prover.rs`, add your program to `create_default_registry()`:

```rust
fn create_default_registry() -> GuestProgramRegistry {
    let mut reg = GuestProgramRegistry::new("evm-l2");
    reg.register(Arc::new(EvmL2GuestProgram));
    reg.register(Arc::new(MyGuestProgram));  // <-- add here
    reg
}
```

---

## Dynamic ELF Loading (No Recompilation)

For programs whose ELF binaries are available as files on disk (e.g., downloaded from the Guest Program Store), use `DynamicGuestProgram`:

### From a directory

```rust
use ethrex_guest_program::programs::dynamic::DynamicGuestProgram;

// Directory layout: /opt/elfs/my-program/sp1/elf, /opt/elfs/my-program/risc0/elf, ...
let program = DynamicGuestProgram::from_dir(
    "my-program",
    10,  // program_type_id
    "/opt/elfs/my-program",
)?;

// Register it
registry.register(Arc::new(program));
```

### With the builder

```rust
use ethrex_guest_program::programs::dynamic::DynamicGuestProgram;
use ethrex_guest_program::traits::backends;

let program = DynamicGuestProgram::builder("my-program", 10)
    .elf_from_file(backends::SP1, "/path/to/sp1.elf")?
    .elf_from_file(backends::RISC0, "/path/to/risc0.elf")?
    .vk_from_bytes(backends::RISC0, risc0_image_id.to_vec())
    .build();
```

### From raw bytes

```rust
let elf_bytes: Vec<u8> = download_elf_from_store("my-program", "sp1").await?;

let program = DynamicGuestProgram::builder("my-program", 10)
    .elf_from_bytes(backends::SP1, elf_bytes)?
    .build();
```

ELF header validation (magic number, RISC-V class, machine type) is performed by default.  To skip it:

```rust
let program = DynamicGuestProgram::builder("my-program", 10)
    .skip_validation()
    .elf_from_bytes(backends::SP1, raw_bytes)?
    .build();
```

---

## The `GuestProgram` Trait Reference

```rust
pub trait GuestProgram: Send + Sync {
    /// Unique ID (e.g., "evm-l2", "zk-dex").
    fn program_id(&self) -> &str;

    /// ELF binary for a backend. Returns None if unsupported.
    fn elf(&self, backend: &str) -> Option<&[u8]>;

    /// Verification key bytes for a backend.
    fn vk_bytes(&self, backend: &str) -> Option<Vec<u8>>;

    /// L1 program type identifier.
    fn program_type_id(&self) -> u8;

    /// Serialize raw input bytes (default: pass-through).
    fn serialize_input(&self, raw: &[u8]) -> Result<Vec<u8>, GuestProgramError>;

    /// Encode raw output bytes for L1 (default: pass-through).
    fn encode_output(&self, raw: &[u8]) -> Result<Vec<u8>, GuestProgramError>;

    /// Validate ELF header (default: magic + class + machine check).
    fn validate_elf(&self, backend: &str, elf: &[u8]) -> Result<(), GuestProgramError>;
}
```

### Backend Constants

Use `backends::SP1`, `backends::RISC0`, `backends::ZISK`, `backends::OPENVM`, `backends::EXEC` as the `backend` parameter.

### Error Types

```rust
pub enum GuestProgramError {
    Serialization(String),
    UnsupportedBackend(String),
    InvalidElf(String),
    Internal(String),
}
```

---

## Testing

Every guest program should have tests covering:

| Test | What it verifies |
|------|------------------|
| `program_id_is_correct` | ID matches the string used in registry |
| `program_type_id_is_correct` | Unique integer for L1 |
| `unsupported_backend_returns_none` | Non-existent backends → None |
| `serialize_input_is_identity` | Pass-through serialization |
| `execution_produces_deterministic_output` | Same input → same output |
| `execution_rejects_empty_input` | Error on empty/invalid input |
| `output_encode_length` | Byte layout matches L1 expectation |
| `rkyv_roundtrip` | Serialize → deserialize preserves data |

Run tests:

```bash
# All guest-program tests
cargo test -p ethrex-guest-program

# All prover tests (includes registry integration)
cargo test -p ethrex-prover

# Both
cargo test -p ethrex-guest-program -p ethrex-prover
```

---

## Existing Programs Reference

| Program | ID | Type ID | Description |
|---------|-----|---------|-------------|
| `EvmL2GuestProgram` | `evm-l2` | 1 | Default EVM-L2 block execution |
| `ZkDexGuestProgram` | `zk-dex` | 2 | Privacy-preserving DEX transfers |
| `TokammonGuestProgram` | `tokamon` | 3 | Location-based reward game |

---

## Checklist

Before submitting a new guest program:

- [ ] Types derive `rkyv` (`Archive`, `RSerialize`, `RDeserialize`) and `serde` traits
- [ ] Execution function is deterministic (no randomness, no system calls)
- [ ] Output `encode()` matches the L1 verifier's expected byte layout
- [ ] `program_type_id` is unique across all registered programs
- [ ] SP1 binary compiles with `#![no_main]` and `sp1_zkvm::entrypoint!`
- [ ] All 8 standard tests pass
- [ ] Program registered in `create_default_registry()` (or loaded dynamically)
- [ ] ELF constants added to `lib.rs` and `build.rs` (for compile-time embedding)
