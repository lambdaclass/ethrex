# Plan: Extract Guest Program to Dedicated `ethrex-guest` Crate

## Executive Summary

This plan outlines the extraction of the guest program from its current location (`crates/l2/prover/src/guest_program/`) to a dedicated top-level crate (`crates/ethrex-guest/`).

**Base Branch**: `refactor_guest_program_structure` (PR #5818)

PR #5818 has already refactored the guest program to separate L1/L2 logic, which significantly simplifies this extraction.

---

## Current State (After PR #5818)

### New Structure in PR #5818

```
crates/l2/prover/src/guest_program/
├── Cargo.toml
├── build.rs
├── src/
│   ├── lib.rs              # Feature-based re-exports + ELF embedding
│   ├── methods.rs          # RISC0 stub
│   │
│   ├── common/             # Shared execution logic (NO feature flags)
│   │   ├── mod.rs
│   │   ├── error.rs        # ExecutionError
│   │   └── execution.rs    # execute_blocks() with VM factory pattern
│   │
│   ├── l1/                 # L1 program (NO feature flags)
│   │   ├── mod.rs
│   │   ├── input.rs        # ProgramInput { blocks, execution_witness }
│   │   ├── output.rs       # ProgramOutput { state hashes, block hash }
│   │   └── program.rs      # execution_program() using Evm::new_for_l1()
│   │
│   ├── l2/                 # L2 program (NO feature flags)
│   │   ├── mod.rs
│   │   ├── input.rs        # ProgramInput { ..., blob_*, fee_configs }
│   │   ├── output.rs       # ProgramOutput { ..., message roots, balance diffs }
│   │   ├── program.rs      # execution_program() using Evm::new_for_l2()
│   │   ├── messages.rs     # Message extraction and digest computation
│   │   ├── blobs.rs        # KZG blob verification
│   │   └── error.rs        # L2ExecutionError
│   │
│   ├── sp1/                # SP1 backend binary
│   ├── risc0/              # RISC0 backend binary
│   ├── zisk/               # ZisK backend binary
│   └── openvm/             # OpenVM backend binary
```

### Key Architectural Improvements in PR #5818

1. **VM Factory Pattern**: `execute_blocks()` takes a closure to create the VM
   ```rust
   execute_blocks(&blocks, execution_witness, elasticity_multiplier, |db, i| {
       // L1: Evm::new_for_l1(db.clone())
       // L2: Evm::new_for_l2(db.clone(), fee_config)
   })
   ```

2. **Clean Separation**: L1 and L2 modules have NO `#[cfg(feature = "l2")]` scattered through them

3. **Backward-Compatible Re-exports**: `lib.rs` uses feature flags only for re-exports
   ```rust
   #[cfg(feature = "l2")]
   pub mod input { pub use crate::l2::ProgramInput; }
   #[cfg(not(feature = "l2"))]
   pub mod input { pub use crate::l1::ProgramInput; }
   ```

---

## EIP-8025 Context

### What EIP-8025 Expects

The consensus-specs define execution proofs that verify block validity:

```python
class PublicInput(Container):
    new_payload_request_root: Root

class ExecutionProof(Container):
    proof_data: ByteList[MAX_PROOF_SIZE]
    proof_type: ProofType
    public_input: PublicInput
```

### ere-guests Integration

The `eth-act/ere-guests` project uses ethrex for **L1 stateless validation**:

```rust
// From ere-guests stateless-validator-ethrex
use ethrex_guest_program::{execution::execution_program, input::ProgramInput};

impl Guest for StatelessValidatorEthrexGuest {
    fn compute<P: Platform>(input: GuestInput<Self>) -> GuestOutput<Self> {
        let res = execution_program(program_input);
        StatelessValidatorOutput::new(new_payload_request_root, res.is_ok())
    }
}
```

**Key insight**:
- **Only the L1 guest** needs to be compliant with ere-guests/EIP-8025
- **L2 guest** is internal to ethrex and can have whatever structure we need

---

## Proposed Structure

```
crates/ethrex-guest/
├── Cargo.toml                    # Workspace manifest
├── README.md
│
├── common/                       # Shared execution logic
│   ├── Cargo.toml               # name = "ethrex-guest-common"
│   └── src/
│       ├── lib.rs
│       ├── error.rs
│       └── execution.rs
│
├── l1/                           # L1 stateless validation (ere-guests compatible)
│   ├── Cargo.toml               # name = "ethrex-guest-program"
│   └── src/
│       ├── lib.rs
│       ├── input.rs
│       ├── output.rs
│       └── program.rs
│
├── l2/                           # L2 stateless validation (ethrex internal)
│   ├── Cargo.toml               # name = "ethrex-guest-l2"
│   └── src/
│       ├── lib.rs
│       ├── input.rs
│       ├── output.rs
│       ├── program.rs
│       ├── messages.rs
│       └── blobs.rs
│
└── bin/                          # Backend binaries
    ├── sp1/
    │   ├── Cargo.toml           # name = "ethrex-guest-sp1"
    │   └── src/main.rs
    ├── risc0/
    │   ├── Cargo.toml           # name = "ethrex-guest-risc0"
    │   └── src/main.rs
    ├── zisk/
    │   ├── Cargo.toml           # name = "ethrex-guest-zisk"
    │   └── src/main.rs
    └── openvm/
        ├── Cargo.toml           # name = "ethrex-guest-openvm"
        ├── openvm.toml
        └── src/main.rs
```

### Package Names

| Directory | Package Name | Notes |
|-----------|--------------|-------|
| `common/` | `ethrex-guest-common` | Shared execution logic |
| `l1/` | `ethrex-guest-program` | ere-guests compatible |
| `l2/` | `ethrex-guest-l2` | Internal, no constraints |
| `bin/sp1/` | `ethrex-guest-sp1` | SP1 binary |
| `bin/risc0/` | `ethrex-guest-risc0` | RISC0 binary |
| `bin/zisk/` | `ethrex-guest-zisk` | ZisK binary |
| `bin/openvm/` | `ethrex-guest-openvm` | OpenVM binary |

**Note**: Only `l1/` (package `ethrex-guest-program`) needs ere-guests compatibility. Everything else is internal to ethrex.

---

## Migration Steps

### Phase 1: Create Crate Structure (based on PR #5818)

1. **Create workspace**
   ```bash
   mkdir -p crates/ethrex-guest
   ```

2. **Create workspace Cargo.toml**
   ```toml
   [workspace]
   members = [
       "common",
       "l1",
       "l2",
       "bin/sp1",
       "bin/risc0",
       "bin/zisk",
       "bin/openvm",
   ]
   resolver = "2"

   [workspace.package]
   edition = "2024"
   license = "MIT"
   rust-version = "1.85"
   version = "0.1.0"

   [workspace.dependencies]
   # ethrex deps
   ethrex-common = { path = "../common/common", default-features = false }
   ethrex-vm = { path = "../vm/vm", default-features = false }
   ethrex-blockchain = { path = "../blockchain/blockchain", default-features = false }
   ethrex-rlp = { path = "../common/rlp", default-features = false }
   ethrex-storage = { path = "../storage/storage", default-features = false }
   ethrex-trie = { path = "../storage/trie", default-features = false }
   ethrex-l2-common = { path = "../l2/common", default-features = false }

   # Local
   ethrex-guest-common = { path = "common" }
   ethrex-guest-program = { path = "l1" }
   ethrex-guest-l2 = { path = "l2" }

   # Serialization
   rkyv = { version = "0.8", default-features = false }
   serde = { version = "1.0", default-features = false, features = ["derive"] }
   ```

3. **Move common code**
   - `src/common/` → `common/src/`

4. **Move L1 code**
   - `src/l1/` → `l1/src/`

5. **Move L2 code**
   - `src/l2/` → `l2/src/`

6. **Move backend binaries**
   - `src/{sp1,risc0,zisk,openvm}/` → `bin/{sp1,risc0,zisk,openvm}/`

### Phase 2: Update Cargo.toml Files

7. **common/Cargo.toml**
   ```toml
   [package]
   name = "ethrex-guest-common"
   version.workspace = true
   edition.workspace = true

   [dependencies]
   ethrex-common = { workspace = true }
   ethrex-vm = { workspace = true }
   ethrex-blockchain = { workspace = true }
   thiserror = { version = "2.0", default-features = false }
   ```

8. **l1/Cargo.toml**
   ```toml
   [package]
   name = "ethrex-guest-program"
   version.workspace = true
   edition.workspace = true

   [dependencies]
   ethrex-guest-common = { workspace = true }
   ethrex-common = { workspace = true }
   ethrex-vm = { workspace = true }
   rkyv = { workspace = true }
   serde = { workspace = true }
   ```

9. **l2/Cargo.toml**
   ```toml
   [package]
   name = "ethrex-guest-l2"
   version.workspace = true
   edition.workspace = true

   [dependencies]
   ethrex-guest-common = { workspace = true }
   ethrex-common = { workspace = true }
   ethrex-vm = { workspace = true }
   ethrex-l2-common = { workspace = true }
   rkyv = { workspace = true }
   serde = { workspace = true }
   ```

### Phase 3: Update Prover Dependency

10. **Update crates/l2/prover/Cargo.toml**
    ```toml
    # Before (in PR #5818)
    guest_program = { path = "./src/guest_program" }

    # After
    ethrex-guest-common = { path = "../../ethrex-guest/common" }
    ethrex-guest-program = { path = "../../ethrex-guest/l1" }
    ethrex-guest-l2 = { path = "../../ethrex-guest/l2" }
    ```

11. **Update backend imports in prover**
    ```rust
    // Before
    use guest_program::input::ProgramInput;
    use guest_program::output::ProgramOutput;
    use guest_program::execution::execution_program;

    // After (for L2 prover)
    use ethrex_guest_l2::{ProgramInput, ProgramOutput, execution_program};
    ```

### Phase 4: Build System

12. **Simplify build.rs**
    - Move ELF embedding to respective backend crates
    - Each backend crate handles its own build
    - Consider a top-level Makefile for orchestration

13. **Backend Cargo.toml example (bin/sp1/Cargo.toml)**
    ```toml
    [package]
    name = "ethrex-guest-sp1"
    version.workspace = true

    [[bin]]
    name = "zkvm-sp1-program"
    path = "src/main.rs"

    [dependencies]
    sp1-zkvm = "5.0"
    ethrex-guest-l2 = { workspace = true }
    ethrex-vm = { workspace = true, features = ["sp1"] }
    rkyv = { workspace = true }

    [features]
    l2 = []
    default = ["l2"]

    [patch.crates-io]
    # SP1 patches...
    ```

### Phase 5: Cleanup & Documentation

14. **Remove old directory**
    ```bash
    rm -rf crates/l2/prover/src/guest_program/
    ```

15. **Update root Cargo.toml**
    - Add `crates/ethrex-guest/*` to workspace members

16. **Create README.md** in `crates/ethrex-guest/`

### Phase 6: CI Workflow Updates

The following CI workflows contain hardcoded paths to `guest_program` that need updating:

#### 17. `.github/workflows/pr-main_l2.yaml`

**Lines 91-92** - Stub SP1 ELF for L2 tests:
```yaml
# Before
mkdir -p crates/l2/prover/src/guest_program/src/sp1/out
touch crates/l2/prover/src/guest_program/src/sp1/out/riscv32im-succinct-zkvm-elf

# After
mkdir -p crates/ethrex-guest/bin/sp1/out
touch crates/ethrex-guest/bin/sp1/out/riscv32im-succinct-zkvm-elf
```

**Lines 340, 654** - Stub verification keys (within `crates/l2/` context):
```yaml
# Before
mkdir -p prover/src/guest_program/src/sp1/out && touch prover/src/guest_program/src/sp1/out/riscv32im-succinct-zkvm-vk-bn254 && touch prover/src/guest_program/src/sp1/out/riscv32im-succinct-zkvm-vk-u32

# After
mkdir -p ../../ethrex-guest/bin/sp1/out && touch ../../ethrex-guest/bin/sp1/out/riscv32im-succinct-zkvm-vk-bn254 && touch ../../ethrex-guest/bin/sp1/out/riscv32im-succinct-zkvm-vk-u32
```

#### 18. `.github/workflows/main_prover.yaml`

**Line 68** - List SP1 output directory:
```yaml
# Before
ls -lah crates/l2/prover/src/guest_program/src/sp1/out/

# After
ls -lah crates/ethrex-guest/bin/sp1/out/
```

#### 19. `.github/workflows/pr-main_l1.yaml`

**Line 115** - Exclude package from tests:
```yaml
# Before
cargo test --workspace --exclude 'ethrex-l2*' --exclude ethrex-prover --exclude guest_program

# After
cargo test --workspace --exclude 'ethrex-l2*' --exclude ethrex-prover --exclude 'ethrex-guest-*'
```

#### 20. `.github/workflows/pr-main_l2_prover.yaml`

**Line 104** - Check Cargo.lock changes:
```yaml
# Before
git diff --exit-code -- crates/l2/prover/src/guest_program/${{ matrix.backend }}/Cargo.lock

# After
git diff --exit-code -- crates/ethrex-guest/bin/${{ matrix.backend }}/Cargo.lock
```

#### 21. `.github/workflows/tag_release.yaml`

**Lines 132-134** - Move verification keys:
```yaml
# Before
mv crates/l2/prover/src/guest_program/src/risc0/out/riscv32im-risc0-vk verification_keys/ethrex-riscv32im-risc0-vk
mv crates/l2/prover/src/guest_program/src/sp1/out/riscv32im-succinct-zkvm-vk-bn254 verification_keys/ethrex-riscv32im-succinct-zkvm-vk-bn254
mv crates/l2/prover/src/guest_program/src/sp1/out/riscv32im-succinct-zkvm-vk-u32 verification_keys/ethrex-riscv32im-succinct-zkvm-vk-u32

# After
mv crates/ethrex-guest/bin/risc0/out/riscv32im-risc0-vk verification_keys/ethrex-riscv32im-risc0-vk
mv crates/ethrex-guest/bin/sp1/out/riscv32im-succinct-zkvm-vk-bn254 verification_keys/ethrex-riscv32im-succinct-zkvm-vk-bn254
mv crates/ethrex-guest/bin/sp1/out/riscv32im-succinct-zkvm-vk-u32 verification_keys/ethrex-riscv32im-succinct-zkvm-vk-u32
```

**Line 197** - Build guest program package:
```yaml
# Before
cargo build --release --package guest_program --features "${{ matrix.zkvm }},ci"

# After
cargo build --release --package ethrex-guest-${{ matrix.zkvm }} --features "ci"
```

**Lines 200-202** - Move SP1 ELF and verification keys:
```yaml
# Before
mv crates/l2/prover/src/guest_program/src/${{ matrix.zkvm }}/out/riscv32im-succinct-zkvm-elf ethrex-riscv32im-${{ matrix.zkvm }}-elf-${{ github.ref_name }}
mv crates/l2/prover/src/guest_program/src/${{ matrix.zkvm }}/out/riscv32im-succinct-zkvm-vk-bn254 ${{ matrix.zkvm }}_verification_keys/...
mv crates/l2/prover/src/guest_program/src/${{ matrix.zkvm }}/out/riscv32im-succinct-zkvm-vk-u32 ${{ matrix.zkvm }}_verification_keys/...

# After
mv crates/ethrex-guest/bin/${{ matrix.zkvm }}/out/riscv32im-succinct-zkvm-elf ethrex-riscv32im-${{ matrix.zkvm }}-elf-${{ github.ref_name }}
mv crates/ethrex-guest/bin/${{ matrix.zkvm }}/out/riscv32im-succinct-zkvm-vk-bn254 ${{ matrix.zkvm }}_verification_keys/...
mv crates/ethrex-guest/bin/${{ matrix.zkvm }}/out/riscv32im-succinct-zkvm-vk-u32 ${{ matrix.zkvm }}_verification_keys/...
```

**Lines 205-206** - Move RISC0 ELF and verification key:
```yaml
# Before
mv crates/l2/prover/src/guest_program/src/${{ matrix.zkvm }}/out/riscv32im-risc0-elf ethrex-riscv32im-${{ matrix.zkvm }}-elf-${{ github.ref_name}}
mv crates/l2/prover/src/guest_program/src/${{ matrix.zkvm }}/out/riscv32im-risc0-vk ${{ matrix.zkvm }}_verification_keys/...

# After
mv crates/ethrex-guest/bin/${{ matrix.zkvm }}/out/riscv32im-risc0-elf ethrex-riscv32im-${{ matrix.zkvm }}-elf-${{ github.ref_name}}
mv crates/ethrex-guest/bin/${{ matrix.zkvm }}/out/riscv32im-risc0-vk ${{ matrix.zkvm }}_verification_keys/...
```

**Line 209** - Move ZisK ELF:
```yaml
# Before
mv crates/l2/prover/src/guest_program/src/${{ matrix.zkvm }}/out/riscv64ima-zisk-elf ethrex-riscv64ima-${{ matrix.zkvm }}-elf-${{ github.ref_name}}

# After
mv crates/ethrex-guest/bin/${{ matrix.zkvm }}/out/riscv64ima-zisk-elf ethrex-riscv64ima-${{ matrix.zkvm }}-elf-${{ github.ref_name}}
```

#### CI Workflow Summary

| Workflow | Changes Required |
|----------|------------------|
| `pr-main_l2.yaml` | 3 path updates (stub ELF/VK creation) |
| `main_prover.yaml` | 1 path update (ls command) |
| `pr-main_l1.yaml` | 1 exclude pattern update |
| `pr-main_l2_prover.yaml` | 1 path update (Cargo.lock diff) |
| `tag_release.yaml` | 8+ path updates (ELF/VK moves, package name) |

---

## ere-guests Compatibility (L1 Only)

**Only the L1 guest needs to be compatible with ere-guests.** The L2 guest is internal to ethrex.

### Current Problem

The current ethrex `ProgramInput` **forces** ere-guests to pass unnecessary fields:

```rust
// Current ethrex ProgramInput (forces unnecessary fields)
pub struct ProgramInput {
    pub blocks: Vec<Block>,
    pub execution_witness: ExecutionWitness,
    pub elasticity_multiplier: u64,           // ere-guests must provide this
    pub fee_configs: Option<Vec<FeeConfig>>,  // ere-guests must provide this
    #[cfg(feature = "l2")]
    pub blob_commitment: ...,
    #[cfg(feature = "l2")]
    pub blob_proof: ...,
}
```

ere-guests doesn't actually need `elasticity_multiplier` or `fee_configs` for L1 validation - they're forced to pass them because the current struct requires it.

### PR #5818 Fixes This

**PR #5818 L1 `ProgramInput` (what ere-guests actually needs):**
```rust
pub struct ProgramInput {
    pub blocks: Vec<Block>,
    pub execution_witness: ExecutionWitness,
}
```

This is correct:
- `elasticity_multiplier`: Uses constant `ELASTICITY_MULTIPLIER = 2` internally
- `fee_configs`: Not needed for L1 (L2-specific)

### ere-guests Benefits

After PR #5818, ere-guests can simplify their code:

```rust
// BEFORE (forced to pass unnecessary fields)
let input = ProgramInput {
    blocks: vec![block],
    execution_witness: input.execution_witness,
    elasticity_multiplier: input.elasticity_multiplier,
    fee_configs: input.fee_configs,
};

// AFTER (clean interface)
let input = ProgramInput {
    blocks: vec![block],
    execution_witness: input.execution_witness,
};
```

### Cargo.toml

```toml
# ere-guests (NO l2 feature needed)
ethrex-guest-program = { git = "...", package = "ethrex-guest-program" }
```

### Migration

- [ ] Coordinate with ere-guests maintainers
- [ ] This is a **simplification** for them, not a burden
- [ ] They can remove `elasticity_multiplier` and `fee_configs` from `StatelessValidatorEthrexInput`

---

## Dependency Graph (After Migration)

```
ethrex-prover
├── ethrex-guest-l2 (for L2 proving)
│   ├── ethrex-guest-common
│   │   ├── ethrex-common
│   │   ├── ethrex-vm
│   │   └── ethrex-blockchain
│   ├── ethrex-l2-common
│   └── ...
│
└── ethrex-guest-{sp1,risc0,zisk,openvm}
    ├── ethrex-guest-l2
    └── zkvm-specific-deps

ere-guests (external)
└── ethrex-guest-program (from l1/)
    └── ethrex-guest-common
```

---

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| ere-guests code update needed | Low | This is a simplification - they remove fields, not add them |
| Package name change | Medium | Keep `package = "ethrex-guest-program"` for L1 crate |
| Build system complexity | Medium | Extract to Makefile, test each backend |
| L2 changes affecting L1 | Low | Clear separation means L2 can evolve independently |
| CI workflow breakage | Medium | Update all 5 affected workflows; test on PR before merge |
| Release artifact paths | High | Carefully verify `tag_release.yaml` changes; test with dry-run |

---

## Testing Checklist

- [ ] L1 program compiles and tests pass
- [ ] L2 program compiles and tests pass
- [ ] Each backend (SP1, RISC0, ZisK, OpenVM) builds successfully
- [ ] Prover integration tests pass
- [ ] ere-guests can depend on the new crate structure
- [ ] CI workflows updated and passing:
  - [ ] `pr-main_l2.yaml` - L2 tests with stub ELFs
  - [ ] `main_prover.yaml` - Prover build verification
  - [ ] `pr-main_l1.yaml` - L1 tests excluding guest packages
  - [ ] `pr-main_l2_prover.yaml` - Cargo.lock diff checks
  - [ ] `tag_release.yaml` - Release artifact paths

---

## References

- [PR #5818](https://github.com/lambdaclass/ethrex/pull/5818) - Base branch with L1/L2 separation
- [EIP-8025](https://eips.ethereum.org/EIPS/eip-8025) - Optional Execution Proofs
- [consensus-specs PR #4828](https://github.com/ethereum/consensus-specs/pull/4828) - Proof engine spec
- [ere-guests PR #7](https://github.com/eth-act/ere-guests/pull/7) - ethrex integration
