# Adding a New App with Fixture Tests

This guide walks through adding a new app to the ethrex L2 platform and setting up
offline fixture-based tests for it.

## Prerequisites

- Rust toolchain (see root `rust-toolchain.toml`)
- Docker + Docker Compose
- Python 3 (for `merge-fixtures.sh`)
- A running deployment or the ability to deploy via the platform

## Quick Start (Fixtures Only)

If your app is **already registered** (evm-l2, zk-dex, tokamon) and you just need
to add fixture tests:

```bash
# 1. Collect fixtures from a running deployment
#    (see fixture-data-collection.md for full details)

# 2. Merge committer + prover data
cd crates/guest-program/tests
./merge-fixtures.sh /tmp/fixtures/<app>/batch_N

# 3. Copy to test directory
mkdir -p fixtures/<app>
cp /tmp/fixtures/<app>/batch_N/fixture.json fixtures/<app>/batch_N_description.json

# 4. Run tests — your app is auto-discovered
cargo test -p ethrex-guest-program --test test_program_output
cargo test -p ethrex-guest-program --test test_commitment_match
cargo test -p ethrex-guest-program --test test_state_continuity
```

That's it. Tests auto-discover all apps under `tests/fixtures/`.

---

## Full Guide: Registering a New App

### Step 1: Assign a Program Type ID

Edit `crates/l2/common/src/lib.rs`:

```rust
pub fn resolve_program_type_id(program_id: &str) -> u8 {
    match program_id {
        "evm-l2" => 1,
        "zk-dex" => 2,
        "tokamon" => 3,
        "my-app" => 4,    // <-- add your app
        _ => 0,
    }
}
```

This is the **single source of truth** for program type IDs.

### Step 2: Implement the Guest Program

Create `crates/guest-program/src/programs/my_app.rs`:

```rust
use crate::traits::{GuestProgram, GuestProgramError, ResourceLimits};

pub struct MyAppGuestProgram;

impl GuestProgram for MyAppGuestProgram {
    fn program_id(&self) -> &str { "my-app" }

    fn elf(&self, backend: &str) -> Option<&[u8]> {
        // Return compiled ELF for each supported backend.
        // Return None for unsupported backends.
        match backend {
            "exec" => Some(include_bytes!("path/to/my_app_elf")),
            _ => None,
        }
    }

    fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> {
        None // SP1 generates VKs at runtime from the ELF
    }

    fn program_type_id(&self) -> u8 { 4 }
}
```

Register it in `crates/guest-program/src/programs/mod.rs`:

```rust
pub mod my_app;
pub use my_app::MyAppGuestProgram;
```

### Step 3: Register in Prover

Edit `crates/l2/prover/src/prover.rs`:

```rust
let all_programs: Vec<(String, Arc<dyn GuestProgram>)> = vec![
    ("evm-l2".to_string(), Arc::new(EvmL2GuestProgram)),
    ("zk-dex".to_string(), Arc::new(ZkDexGuestProgram)),
    ("tokamon".to_string(), Arc::new(TokammonGuestProgram)),
    ("my-app".to_string(), Arc::new(MyAppGuestProgram)),  // <-- add
];
```

### Step 4: Add Platform Database Entry

Edit `platform/server/db/db.js`, add to the `programs` array:

```javascript
{
  programId: "my-app",
  typeId: 4,
  name: "My App",
  category: "defi",
  description: "Description of my app",
}
```

### Step 5: Add Docker Compose Profile

Edit `platform/server/lib/compose-generator.js`, add to `APP_PROFILES`:

```javascript
"my-app": {
  dockerfile: null,           // null = default Dockerfile
  buildFeatures: "--features l2,l2-sql",
  guestPrograms: null,        // or "evm-l2,my-app" if building multiple ELFs
  genesisFile: "l2.json",     // or custom genesis file
  proverBackend: "exec",      // "exec" for execution-only, "sp1" for ZK proofs
  sp1Enabled: false,
  registerGuestPrograms: null, // or "my-app" for SP1
  programsToml: null,          // or "programs-my-app.toml" for SP1
  deployRich: true,
  description: "My App L2",
},
```

### Step 6: Collect Fixtures

See [fixture-data-collection.md](./fixture-data-collection.md) for the full workflow.

Summary:

1. Add `ETHREX_DUMP_FIXTURES=/tmp/fixtures` to docker-compose.yaml (L2 + prover containers)
2. Deploy and generate transactions (E2E test or manual)
3. Wait for batches to be committed and proved
4. Merge: `./merge-fixtures.sh /tmp/fixtures/my-app/batch_N`
5. Copy: `cp /tmp/fixtures/my-app/batch_N/fixture.json crates/guest-program/tests/fixtures/my-app/batch_N.json`

### Step 7: Verify

```bash
cargo test -p ethrex-guest-program --test test_program_output
cargo test -p ethrex-guest-program --test test_commitment_match
cargo test -p ethrex-guest-program --test test_state_continuity
```

All tests auto-discover your new app's fixtures.

---

## Registration Checklist

| Step | File | What to Add |
|------|------|-------------|
| Type ID | `crates/l2/common/src/lib.rs` | Match arm in `resolve_program_type_id()` |
| Guest Program | `crates/guest-program/src/programs/` | Struct implementing `GuestProgram` trait |
| Program mod.rs | `crates/guest-program/src/programs/mod.rs` | `pub mod` + `pub use` |
| Prover registry | `crates/l2/prover/src/prover.rs` | Entry in `all_programs` vec |
| Platform DB | `platform/server/db/db.js` | Entry in `programs` array |
| Compose profile | `platform/server/lib/compose-generator.js` | Entry in `APP_PROFILES` |
| Fixtures | `crates/guest-program/tests/fixtures/<app>/` | JSON files (auto-discovered) |

## Fixture JSON Schema

```json
{
  "app": "my-app",
  "batch_number": 1,
  "program_type_id": 4,
  "chain_id": 65536999,
  "description": "Batch with deposit transaction",
  "prover": {
    "initial_state_hash": "0x...",
    "final_state_hash": "0x...",
    "l1_out_messages_merkle_root": "0x...",
    "l1_in_messages_rolling_hash": "0x...",
    "blob_versioned_hash": "0x...",
    "last_block_hash": "0x...",
    "non_privileged_count": 1,
    "balance_diffs": [],
    "l2_in_message_rolling_hashes": [],
    "encoded_public_values": "0x...",
    "sha256_public_values": "0x..."
  },
  "committer": {
    "new_state_root": "0x...",
    "withdrawals_merkle_root": "0x...",
    "priv_tx_rolling_hash": "0x...",
    "non_privileged_txs": 1,
    "balance_diffs": [],
    "l2_in_message_rolling_hashes": []
  }
}
```

## What Each Test Verifies

| Test | What it checks |
|------|---------------|
| `test_program_output` | `ProgramOutput.encode()` matches prover's `encoded_public_values` byte-for-byte |
| `test_commitment_match` | Committer calldata fields match prover public values (prevents 00e errors) |
| `test_state_continuity` | Batch N `final_state_hash` == Batch N+1 `initial_state_hash` |
| `chain_id_consistent` | All fixtures for an app have the same `chain_id` |
| `program_type_id_consistent` | All fixtures for an app have the same `program_type_id` |

## Directory Structure

```
crates/guest-program/tests/
├── fixtures/
│   ├── zk-dex/                    # Auto-discovered
│   │   ├── batch_2_deposit.json
│   │   └── batch_10_withdraw.json
│   ├── my-app/                    # Auto-discovered
│   │   └── batch_1.json
│   └── evm-l2/                    # Auto-discovered (when fixtures exist)
├── fixture_types.rs               # Generic loader + discover_all_apps()
├── merge-fixtures.sh              # committer.json + prover.json -> fixture.json
├── test_program_output.rs         # Auto-discovery test
├── test_commitment_match.rs       # Auto-discovery test
└── test_state_continuity.rs       # Auto-discovery test
```
