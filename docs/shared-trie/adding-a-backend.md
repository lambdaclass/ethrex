# Adding a New Trie Backend

This guide explains how to add a new trie backend (e.g. a binary trie for
EIP-7864) to ethrex using the shared-trie abstraction layer.

## Overview

The abstraction uses enum dispatch (not `dyn`). Adding a backend means:

1. Implement the backend in a new crate or module
2. Add a variant to the `StateBackend`, `Merkleizer`, and `NodeUpdates` enums
3. Add backend-specific wiring in a new `*_wiring.rs` file in `ethrex-storage`

Shared code (`blockchain.rs`, `vm.rs`, `store.rs`, `payload.rs`) should need
zero changes. The compiler enforces exhaustive matching on all enums.

## Step-by-step

### 1. Create the backend implementation

Create a new module (e.g. `crates/common/binary-trie/`) or add to an existing
crate. The backend struct must implement:

- `StateReader` trait (from `ethrex-state-backend`):
  - `account(addr) -> Option<AccountInfo>`
  - `storage(addr, slot) -> H256`
  - `code(addr, code_hash) -> Option<Vec<u8>>`

- `StateCommitter` trait (from `ethrex-state-backend`):
  - `update_accounts(addrs, muts)`
  - `update_storage(addr, slots)`
  - `clear_storage(addr)`
  - `hash() -> H256`
  - `commit() -> MerkleOutput`

The backend handles key derivation (address/slot hashing), node encoding, and
root computation internally. These never leak into shared code.

### 2. Add enum variants

**`StateBackend`** in `crates/storage/state_backend.rs`:
```rust
pub enum StateBackend {
    Mpt(MptBackend),
    Binary(BinaryBackend),  // new
}
```

Add match arms to all `StateReader`, `StateCommitter`, and inherent method
implementations. The compiler will flag every missing arm.

**`Merkleizer`** in `crates/storage/merkleizer.rs`:
```rust
pub enum Merkleizer {
    Mpt(MptMerkleizer),
    Binary(BinaryMerkleizer),  // new
}
```

**`NodeUpdates`** in `crates/common/state-backend/src/lib.rs`:
```rust
pub enum NodeUpdates {
    Mpt { state_updates, storage_updates },
    Binary { node_diffs: Vec<(Vec<u8>, Vec<u8>)> },  // new
}
```

### 3. Add backend-specific wiring

Create `crates/storage/binary_wiring.rs` (parallel to `mpt_wiring.rs`).
This file contains:

- **Factory methods** on `Store`:
  - `new_binary_state_reader(state_root) -> StateBackend`
  - `new_binary_state_writer() -> StateBackend`
  - `new_binary_witness_recorder(state_root) -> StateBackend`

- **DB table helpers**:
  - `write_binary_node_updates(node_diffs)` -- direct writes to binary trie tables
  - `build_binary_cache_layer(node_diffs) -> Vec<(Vec<u8>, Vec<u8>)>` -- for TrieLayerCache

- **FKV generator** (if applicable):
  - `binary_flatkeyvalue_generator(...)` -- background thread for flat state tables

- **Proof methods** (for `eth_getProof` RPC):
  - `get_binary_account_proof(state_root, addr, keys) -> AccountProof`

### 4. Wire into dispatch points

The following places in `store.rs` match on `NodeUpdates` and need a new arm:

- `write_node_updates_direct()` -- dispatches to `write_binary_node_updates()`
- `apply_trie_updates()` -- dispatches to `build_binary_cache_layer()`

Associated functions on `StateBackend` in `state_backend.rs` also match on
`BackendKind` and need a new arm each:

- `StateBackend::compute_genesis_root(kind, genesis)` -- dispatches to the
  backend's `genesis_root` free function (e.g.
  `ethrex_binary_trie::genesis_root`)
- `StateBackend::compute_genesis_block(kind, genesis)` -- dispatches to the
  backend's `genesis_block` free function

These are pure functions (no `&self`); they are the canonical entry points
for CLI tooling, deployers, and startup code that have a `Genesis` but no
`Store`. The compiler will flag every missing arm when you add
`BackendKind::Binary`.

The `Store::from_backend()` factory needs to know which backend to use. Add a
configuration field or feature flag to select the active backend.

### 5. Witness generation

`StateBackend` has witness methods that each backend implements:

- `init_witness()` -- set up recording state
- `record_witness_accesses(store, parent_hash, access_info)` -- log pre-state reads
- `apply_updates_with_witness_state(updates) -> MerkleOutput` -- apply with logging
- `advance_witness_to(store, block_hash)` -- advance to next block
- `finalize_witness(touched_accounts) -> Vec<Vec<u8>>` -- serialize proof data

The binary backend returns its own proof format in `state_proof: Vec<Vec<u8>>`.
`ExecutionWitness` is backend-agnostic (just bytes).

### 6. Guest program

The zkVM guest program holds the backend type directly (no `StateBackend` enum).
For binary trie: create `BinaryGuestProgramState` that deserializes
`ExecutionWitness.state_proof` in the binary format, builds the binary trie,
and implements `VmDatabase`.

### 7. Layer cache

`TrieLayerCache` (in `crates/storage/layering.rs`) is format-agnostic: it stores and retrieves
opaque byte key-value pairs. The format of those bytes is entirely determined by the backend that
writes into the cache via `build_*_cache_layer`.

For the MPT backend, the consumer is `MptTrieWrapper` (in `mpt_wiring.rs`), which implements
`ethrex_trie::TrieDB` and expects MPT-nibble-encoded keys as produced by `mpt_apply_prefix`.

To add a new backend:

- Provide your own `TrieDB`-equivalent struct that reads from `TrieLayerCache` using your
  backend-specific key encoding (see `MptTrieWrapper` in `mpt_wiring.rs` for the MPT pattern).
- Do **not** reuse `mpt_apply_prefix`; define a parallel `<backend>_apply_prefix` in your own
  wiring file.

### 8. DB tables

Add new tables for the binary trie's node storage:

```rust
pub static BINARY_TRIE_NODES: &str = "BinaryTrieNodes";
```

#### Flat state tables and the format discriminator

`ACCOUNT_FLATKEYVALUE` and `STORAGE_FLATKEYVALUE` are shared across backends;
their keys are MPT nibble-path bytes in the current implementation.

A single-byte format discriminator stored in `MISC_VALUES` under the key
`state_backend_format` (see `STATE_BACKEND_FORMAT_KEY` in `tables.rs`) records
which format is on disk. The byte mapping is:

| Byte | Format |
|------|--------|
| `0`  | MPT nibble-path (current default) |

`Store::from_backend` enforces this contract:

- **First open** (key absent): writes the byte for the active `BackendKind`.
- **Subsequent opens** (key present): compares the on-disk byte to the
  configured `BackendKind`; returns `StoreError::Custom("state backend format
  mismatch: ...")` on disagreement.

When adding a new backend you MUST:

(a) Claim a unique byte value and document it in `tables.rs` alongside the
    existing table.
(b) Either reuse the MPT nibble-path format (writing `0`) or document a
    migration / parallel-tables strategy and register a new byte value.

The mismatch guard prevents a store written by one backend from being silently
read by another with an incompatible key format.

## What you do NOT need to change

- `blockchain.rs` -- orchestration only, no trie types
- `vm.rs` -- reads through `StateReader` trait, no backend downcasts
- `payload.rs` -- uses `Store::apply_account_updates_batch` (backend-agnostic)
- `store.rs` -- only needs new `NodeUpdates` match arms (mechanical)

## Snap sync considerations

Snap sync (in `crates/networking/p2p/sync/`) is currently tightly coupled to
MPT. The Ethereum snap sync protocol is MPT-specific: it uses account range
proofs, storage range proofs, trie node requests, and trie healing, all
defined in terms of MPT node hashes and paths.

A binary trie would need its own sync protocol. Two approaches:

### Option A: Per-backend sync implementations

Each backend provides its own sync module. The P2P layer dispatches based on
the active backend (or the peer's advertised capabilities):

```
crates/networking/p2p/sync/
    mpt_snap_sync.rs    -- current MPT snap sync
    binary_sync.rs      -- new binary trie sync
    mod.rs              -- dispatch
```

Shared infrastructure (downloading, progress tracking, retry logic) stays in
`mod.rs`. Backend-specific logic (proof verification, trie healing, range
requests) goes in the per-backend files.

### Option B: Protocol negotiation

If binary and MPT nodes coexist on the network during a transition period:

1. Peers advertise which backends they support via the `eth` protocol handshake
2. The sync layer picks the matching protocol
3. During transition, a `TransitionBackend` reads from MPT (base) and writes
   to binary (overlay)

### Current state

MPT snap sync code in `mpt_wiring.rs` uses `store.open_direct_state_trie()`,
`store.iter_accounts()`, `AccountState::decode().storage_root`, and other
MPT-specific operations. These are already isolated in `mpt_wiring.rs` and not
in shared code paths.

When adding a binary backend, the snap sync code does NOT need to be
abstracted -- just provide a parallel implementation. The MPT snap sync
continues to work for MPT state. Binary sync is added alongside it.

## Transition backend (fork activation)

For a hard fork that switches from MPT to binary trie:

```rust
pub enum StateBackend {
    Mpt(MptBackend),
    Binary(BinaryBackend),
    Transition(TransitionBackend),  // reads from MPT, writes to binary
}
```

The `TransitionBackend`:
- Read path: check binary overlay first, fall back to MPT base
- Write path: binary only
- Designed to be deletable once migration completes

The fork activation gate checks `block.timestamp >= fork_timestamp` to decide
which backend to use for new blocks. Historical blocks always use the backend
they were created with.
