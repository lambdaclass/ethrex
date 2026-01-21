# Snap Sync Refactoring Plan

## Overview

The Snap Sync implementation spans ~6,500 lines across 7 files. This plan provides a structured approach to simplify and improve the code.

## Current Status

| Phase | Status | Risk Level |
|-------|--------|------------|
| Phase 1: Foundation | Completed | Low |
| Phase 2: Protocol Layer | Completed | Medium |
| Phase 3: Healing Unification | In Progress | Medium-High |
| Phase 4: Sync Orchestration | Pending | High |
| Phase 5: Error Handling | Pending | Medium |

## Files Involved

### Original Structure
| File | Lines | Purpose |
|------|-------|---------|
| `crates/networking/p2p/snap.rs` | 1,008 | Server-side request processing (90% tests) |
| `crates/networking/p2p/rlpx/snap.rs` | 389 | Protocol message definitions |
| `crates/networking/p2p/sync.rs` | 1,648 | Main sync orchestration |
| `crates/networking/p2p/sync/state_healing.rs` | 471 | State trie healing |
| `crates/networking/p2p/sync/storage_healing.rs` | 718 | Storage healing |
| `crates/networking/p2p/sync/code_collector.rs` | 102 | Bytecode collection |
| `crates/networking/p2p/peer_handler.rs` | 2,074 | Client-side snap requests (~800 lines snap-related) |

### New Structure (After Phases 1-2)
| File | Purpose |
|------|---------|
| `crates/networking/p2p/snap/mod.rs` | Snap module re-exports |
| `crates/networking/p2p/snap/server.rs` | Server-side request processing |
| `crates/networking/p2p/snap/constants.rs` | Centralized protocol constants |
| `crates/networking/p2p/rlpx/snap/mod.rs` | Protocol message re-exports |
| `crates/networking/p2p/rlpx/snap/messages.rs` | Message struct definitions |
| `crates/networking/p2p/rlpx/snap/codec.rs` | RLPxMessage implementations |
| `crates/networking/p2p/tests/snap_server_tests.rs` | Snap server tests |

---

## Phase 1: Foundation (Completed)

**Risk Level:** Low

### 1.1 Create snap module directory
```bash
mkdir -p crates/networking/p2p/snap
```

### 1.2 Move server code
- Move `snap.rs` production code to `snap/server.rs`
- Create `snap/mod.rs` with re-exports

### 1.3 Create constants module
Create `snap/constants.rs` with documented constants:
- `MAX_RESPONSE_BYTES`, `SNAP_LIMIT`, `HASH_MAX`
- `RANGE_FILE_CHUNK_SIZE`, `STORAGE_BATCH_SIZE`, `NODE_BATCH_SIZE`
- `BYTECODE_CHUNK_SIZE`, `CODE_HASH_WRITE_BUFFER_SIZE`
- `PEER_REPLY_TIMEOUT`, `PEER_SELECT_RETRY_ATTEMPTS`, `REQUEST_RETRY_ATTEMPTS`
- `MAX_IN_FLIGHT_REQUESTS`, `MAX_HEADER_CHUNK`, `MAX_BLOCK_BODIES_TO_REQUEST`
- `MIN_FULL_BLOCKS`, `EXECUTE_BATCH_SIZE_DEFAULT`, `SECONDS_PER_BLOCK`
- `MISSING_SLOTS_PERCENTAGE`, `MAX_HEADER_FETCH_ATTEMPTS`
- `SHOW_PROGRESS_INTERVAL_DURATION`

### 1.4 Move tests
- Extract test module from `snap.rs` to `tests/snap_server_tests.rs`
- Update test imports to use public API

### 1.5 Update imports
- Update `peer_handler.rs` to re-export constants for backward compatibility
- Update `sync.rs`, `state_healing.rs`, `storage_healing.rs`, `code_collector.rs`

---

## Phase 2: Protocol Layer Cleanup (Completed)

**Risk Level:** Medium

### 2.1 Create rlpx/snap directory
```bash
mkdir -p crates/networking/p2p/rlpx/snap
```

### 2.2 Split snap.rs into modules
- `rlpx/snap/messages.rs` - Message struct definitions
- `rlpx/snap/codec.rs` - RLPxMessage implementations
- `rlpx/snap/mod.rs` - Re-exports

### 2.3 Add message codes module
```rust
pub mod codes {
    pub const GET_ACCOUNT_RANGE: u8 = 0x00;
    pub const ACCOUNT_RANGE: u8 = 0x01;
    // ... etc
}
```

**Note:** Did not implement RLPxMessage macro as originally planned - implementations have variations (e.g., `GetStorageRanges` has special hash handling).

---

## Phase 3: Healing Unification (In Progress)

**Risk Level:** Medium-High

### 3.1 Create healing module directory
```bash
mkdir -p crates/networking/p2p/sync/healing
```

### 3.2 Rename Membatch to PendingNodes
- `MembatchEntryValue` → `PendingNodeEntry`
- `Membatch` → `PendingNodes`
- `MembatchEntry` → `PendingNodeEntry` (in storage_healing)

### 3.3 Create shared healing types
Create `sync/healing/mod.rs` with:
```rust
pub trait HealingProcess {
    fn heal_batch(&mut self, store: &Store) -> Result<HealingProgress, TrieError>;
    fn is_complete(&self) -> bool;
    fn progress(&self) -> HealingProgress;
}

pub struct HealingProgress {
    pub leafs_healed: u64,
    pub roots_healed: u64,
    pub pending_nodes: usize,
}
```

### 3.4 Migrate healing modules
- Move `state_healing.rs` to `healing/state.rs`
- Move `storage_healing.rs` to `healing/storage.rs`
- Create `healing/pending_nodes.rs` for shared types

### 3.5 Update sync.rs imports
```rust
use crate::sync::healing::{heal_state_trie_wrap, heal_storage_trie};
```

---

## Phase 4: Sync Orchestration (Pending)

**Risk Level:** High

### 4.1 Create sync/full.rs
Move from `sync.rs`:
- `sync_cycle_full` function
- `add_blocks_in_batch` function
- `add_blocks` function
- Related helper functions

### 4.2 Create sync/snap_sync.rs
Move from `sync.rs`:
- `snap_sync` / `sync_cycle_snap` function
- `update_pivot` function
- `block_is_stale` function
- `download_accounts` function
- Related snap sync state management

### 4.3 Update sync/mod.rs
Keep:
- `Syncer` struct
- `SyncMode` enum
- `SyncError` enum
- Re-exports from `full.rs` and `snap_sync.rs`

### 4.4 Extract client-side snap requests
Move from `peer_handler.rs` (~800 lines) to `snap/client.rs`:
- `request_account_range` / `request_account_range_worker`
- `request_storage_ranges` / `request_storage_ranges_worker`
- `request_bytecodes`
- `request_state_trienodes` / `request_storage_trienodes`

### 4.5 Update peer_handler.rs
- Remove moved functions
- Import from `snap/client.rs`
- Keep eth protocol functions

---

## Phase 5: Error Handling (Pending)

**Risk Level:** Medium

### 5.1 Create snap/error.rs
```rust
#[derive(Debug, thiserror::Error)]
pub enum SnapError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Protocol(#[from] PeerConnectionError),
    #[error(transparent)]
    Trie(#[from] TrieError),
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Response validation failed: {0}")]
    ValidationError(String),
    #[error("Peer selection failed: {0}")]
    PeerSelection(String),
}
```

### 5.2 Update server functions
- Return `Result<T, SnapError>` instead of mixed error types

### 5.3 Update client functions
- Return `Result<T, SnapError>` instead of `PeerHandlerError`

---

## Implementation Order (Dependencies)

```
Phase 1.1-1.2 (snap module)
    ↓
Phase 1.3 (constants) ──→ Phase 2.1-2.3 (rlpx reorganization)
    ↓
Phase 1.4-1.5 (tests, imports)
    ↓
Phase 3.1-3.2 (pending_nodes)
    ↓
Phase 3.3-3.5 (healing unification)
    ↓
Phase 4.1-4.3 (sync split)
    ↓
Phase 4.4-4.5 (snap client extraction)
    ↓
Phase 5.1-5.3 (error consolidation)
```

---

## Verification Checkpoints

Run after each phase:

1. **Unit tests**: `cargo test -p ethrex-p2p`
2. **Compilation**: `cargo check -p ethrex-p2p`
3. **Lint**: `cargo clippy -p ethrex-p2p`

For protocol changes (Phase 2+):
4. **Hive tests**: Run devp2p snap protocol tests
5. **Integration**: Full snap sync on Sepolia/Hoodi

---

## Risk Mitigation

| Phase | Risk | Mitigation |
|-------|------|------------|
| 1. Foundation | Low | Simple reorganization, APIs unchanged |
| 2. Protocol | Medium | Extensive hive testing |
| 3. Healing | Medium-High | Incremental migration, keep old files until verified |
| 4. Sync orchestration | High | Feature flags, integration tests, gradual extraction |
| 5. Error handling | Medium | Keep old errors, wrap in new type initially |

---

## Notes

### Decisions Made
- **No RLPxMessage macro**: Implementations vary too much (e.g., `GetStorageRanges` special hash handling)
- **Backward-compatible re-exports**: Constants re-exported from `peer_handler.rs` to avoid breaking changes
- **Incremental approach**: Each phase builds on previous, allowing verification at each step

### Key Considerations
- The `accounts_by_root_hash` structure in sync is unbounded - consider adding limits in Phase 4
- Tests should cover edge cases for hash boundary handling in `GetStorageRanges`
- Healing processes share similar patterns but have distinct algorithms - trait unification should preserve this
