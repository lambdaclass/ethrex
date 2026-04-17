# Shared Trie Abstraction -- Specification

## 1. Overview

### Goals

ethrex needs to support both the Merkle Patricia Trie (MPT) and a future binary
trie (EIP-7864) side by side. This spec defines the abstraction layer that makes
that possible without regressing MPT performance or polluting the codebase with
backend-specific concerns.

### Framing

- **MPT is the canonical baseline.** It is the production, battle-tested state
  tree. EIP-7864 is a draft that may never activate on mainnet.
- The abstraction exists to **not break MPT** while leaving room for an
  alternative implementation.
- When a design choice is ambiguous, **pick the shape that makes MPT's hot path
  simplest.** The binary trie is the newcomer and must adapt.

### Design principles

1. **Enum dispatch, not `dyn`.** Monomorphizes MPT hot paths. No vtable on the
   production path.
2. **Merkleization is a backend concern.** The blockchain pipeline calls into
   `StateBackend` methods; it never creates raw `Trie` objects or routes by
   first nibble.
3. **Flat state is trie-agnostic.** The flat state tables are shared across
   backends. The generator uses backend-provided iteration.
4. **Streaming merkleization preserves execution overlap.** The current pipeline
   has execution sending `AccountUpdate` batches over a channel to the
   merkleizer, which processes them concurrently. The new design keeps this.
5. **PR 1 ships the complete framework.** Not a minimal refactor -- the full
   `StateBackend` enum (single `Mpt` arm), all traits, all dispatch. PR 2 just
   adds the `Binary` arm.
6. **No MPT leakage into shared crates.** `ethrex-common` must not depend on
   `ethrex-trie`. Trie-specific types (`Nibbles`, `Node`, `TrieLogger`) are
   never re-exported from common.

---

## 2. Crate Layout

### Dependency graph

```
ethrex-common              Base types: Address, AccountUpdate, AccountInfo,
  (no trie dep!)           Code, BlockHeader, etc.
       ^
       |
ethrex-state-backend       Traits: StateReader, StateCommitter.
  (depends on common)      Types: Account, AccountMut, MerkleOutput, NodeUpdates,
                           StateError. NO concrete backends.
       ^
       |
ethrex-trie                MptBackend, MptMerkleizer (implements traits).
  (depends on              Also: Trie, Nibbles, Node -- MPT internals.
   state-backend,          Uses AccountState, AccountUpdate from common.
   common)                 Does NOT depend on ethrex-storage.
       ^
       |
ethrex-storage             StateBackend enum (assembles backends).
  (depends on all above)   Store, TrieLayerCache, flat state generator,
                           trie update worker. Merkleizer enum lives here.
                           MPT-specific wiring in mpt_wiring.rs (trie
                           opening, hash helpers, genesis, snap sync).
                           store.rs has ZERO MPT-specific code.
```

### Key rules

- `ethrex-common` does NOT depend on `ethrex-trie`. This is a **change from
  the current codebase** (see Section 8, "Breaking ethrex-common -> ethrex-trie").
- `ethrex-state-backend` depends on `ethrex-common` (for `Address`, `H256`,
  `U256`). It does NOT depend on `ethrex-trie` or `ethrex-storage`.
  **Important:** `ethrex-common` must be added to `ethrex-state-backend`'s
  `Cargo.toml` (it is currently missing -- the Phase 2 agent removed it to
  avoid a cycle that the dep inversion now resolves).
- `ethrex-trie` depends on `ethrex-state-backend` and `ethrex-common` (for
  `AccountState`, `AccountUpdate` in `MptMerkleizer`). This is safe because
  `ethrex-common` no longer depends on `ethrex-trie` (the cycle is broken).
  It does NOT depend on `ethrex-storage`.
- `ethrex-vm` adds `ethrex-trie` as a dependency (for `MptBackend`) and
  hosts `GuestProgramState` in `crates/vm/guest_program_state.rs`. The heavy
  deps (`crossbeam`, `rayon`) that `ethrex-trie` pulls in already reach
  `ethrex-vm` and the guest program through other paths (`ethrex-common`,
  `ethrex-levm`), so no feature gating is needed.
- `ethrex-storage` is the assembly point: it depends on all of the above and
  defines the `StateBackend` enum and the `Merkleizer` trait.

### Crate responsibilities

| Crate | Contains | Does NOT contain |
|-------|----------|------------------|
| `ethrex-state-backend` | `StateReader`, `StateCommitter`, `AccountMut`, `CodeMut`, `MerkleOutput`, `NodeUpdates`, `StateError`, `BackendKind`, `CodeReader` | Concrete backends, `Trie`, `Nibbles`, `Merkleizer` enum |
| `ethrex-trie` | `MptBackend`, `MptMerkleizer`, `TrieProvider`, `Trie`, `Nibbles`, `Node`, `compute_*_root` (all five), `validate_block_body`, `genesis_block` / `genesis_root` | `StateBackend` enum, `Store`, `Merkleizer` enum, `GuestProgramState`, `ExecutionWitness` |
| `ethrex-storage` | `StateBackend` enum, `Merkleizer` enum, `Store`, layer cache, flat state generator | Trie internals |
| `ethrex-common` | `AccountUpdate`, `AccountState`, `AccountInfo`, `Code`, block types, `ExecutionWitness` + `RpcExecutionWitness` + their conversions (`From`, `into_execution_witness`) | Trie types (`Trie`, `Nibbles`, `Node`, `TrieLogger`), `compute_*_root`, `validate_block_body` |
| `ethrex-vm` | `GuestProgramState`, `GuestProgramStateError`, `Evm`, `VmDatabase` | `StateBackend` enum |

### Why `Merkleizer` is an enum in `ethrex-storage`, not a trait in `ethrex-state-backend`

The `Merkleizer`'s `feed_updates` method takes `Vec<AccountUpdate>`, which is
defined in `ethrex-common`. `ethrex-state-backend` depends on `ethrex-common`
and can access `AccountUpdate`, so the type is reachable. However, putting a
`Merkleizer` trait in `state-backend` would require `MptMerkleizer` (in
`ethrex-trie`) to implement it. Then `ethrex-storage` would need to construct
a `Box<dyn Merkleizer>` from the concrete type -- requiring `dyn` dispatch on
the hot merkleization path, violating principle 1.

Instead, `Merkleizer` is an enum in `ethrex-storage` that directly holds
`MptMerkleizer` (from `ethrex-trie`). The enum dispatches to each backend's
inherent methods via match -- no trait, no vtable.

```rust
// In ethrex-trie:
impl MptMerkleizer {
    pub fn feed_updates(&mut self, updates: Vec<AccountUpdate>) -> Result<(), StateError> { ... }
    pub fn finalize(self) -> Result<MerkleOutput, StateError> { ... }
}

// In ethrex-storage:
pub enum Merkleizer {
    Mpt(MptMerkleizer),
    // PR 2: Binary(BinaryMerkleizer),
}

impl Merkleizer {
    pub fn feed_updates(&mut self, updates: Vec<AccountUpdate>) -> Result<(), StateError> {
        match self {
            Merkleizer::Mpt(m) => m.feed_updates(updates),
        }
    }
    pub fn finalize(self) -> Result<MerkleOutput, StateError> {
        match self {
            Merkleizer::Mpt(m) => m.finalize(),
        }
    }
}
```

---

## 3. Architecture

### Full pipeline

```
  Execution (VM)
       |
       | Vec<AccountUpdate> over channel (streaming, per-tx or per-batch)
       v
  Merkleizer (backend-specific, via StateBackend enum dispatch)
       |
       | MerkleOutput { root, NodeUpdates, code_updates, accumulated_updates }
       v
  Storage layer
       |
       +---> Trie layer cache (in-memory diff-layers, generic byte KV)
       +---> Disk commit (backend-specific node tables)
       +---> Flat state generator (background thread, shared tables)
```

### Threading model (block execution)

`execute_block_pipeline` in `blockchain.rs` spawns three scoped threads:

1. **Warmer thread** -- prefetches state into caches.
2. **Execution thread** -- runs the EVM, sends `Vec<AccountUpdate>` batches
   over a channel as transactions complete.
3. **Merkleizer thread** -- receives update batches, feeds them into the
   backend's merkleizer, produces `MerkleOutput` on finalize.

After all three join, the pipeline validates the state root and hands the
`MerkleOutput` to the storage layer.

### What changes from the current architecture

Today, `handle_merkleization` in `blockchain.rs` (line ~590) directly:
- Creates 16 crossbeam channels and spawns 16 `handle_subtrie` worker threads
- Routes `AccountUpdate`s to workers by `hashed_address[0] >> 4`
- Each worker holds its own `Trie`, applies storage updates, collects node diffs
- Workers exchange `StorageShard` messages for cross-bucket storage keys
- The main thread assembles 16 `BranchNode` sub-roots into the final state root

**All of this moves inside `MptMerkleizer`.** The blockchain pipeline calls
`StateBackend::new_merkleizer()`, then `feed_updates()` / `finalize()`.

---

## 4. Core Types and Traits

### Types in `ethrex-state-backend`

```rust
// crates/common/state-backend/src/lib.rs

/// Unified account view. No storage_root -- each backend keeps root
/// structure internal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub nonce: u64,
    pub balance: U256,
    pub code_hash: H256,
}

#[derive(Clone, Debug)]
pub struct CodeMut {
    pub code: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct AccountMut {
    pub account: Option<Account>,
    pub code: Option<CodeMut>,
    /// Current total code size. MUST be populated on every mutation.
    /// Binary backends pack this into their on-trie BasicData leaf.
    /// MPT ignores it.
    pub code_size: usize,
}

pub struct CommitOutput {
    pub root: H256,
    pub storage_roots: HashMap<Address, (H256, H256)>,
}

/// Output of merkleization. Lives in ethrex-state-backend.
/// Uses Code and AccountUpdate from ethrex-common (state-backend
/// depends on common).
pub struct MerkleOutput {
    pub root: H256,
    /// Backend-specific node diffs for the storage layer.
    pub node_updates: NodeUpdates,
    /// Code deployments. Uses Code (not raw bytes) to avoid
    /// recomputing jump targets and keccak hash downstream.
    pub code_updates: Vec<(H256, Code)>,
    /// Accumulated account updates for witness pre-computation.
    /// Populated when precompute_witnesses is enabled.
    pub accumulated_updates: Option<Vec<AccountUpdate>>,
}

pub enum NodeUpdates {
    Mpt {
        /// State trie node changes: (nibble_path_bytes, rlp_node).
        /// MptMerkleizer converts from Nibbles to Vec<u8> at its boundary.
        state_updates: Vec<(Vec<u8>, Vec<u8>)>,
        /// Per-account storage trie changes.
        storage_updates: Vec<(H256, Vec<(Vec<u8>, Vec<u8>)>)>,
    },
    // PR 2: Binary { node_diffs: Vec<(Vec<u8>, Vec<u8>)> },
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("trie error: {0}")]
    Trie(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("other: {0}")]
    Other(String),
}
```

### `StateReader` trait

```rust
pub trait StateReader {
    fn account(&self, addr: Address) -> Result<Option<Account>, StateError>;
    fn storage(&self, addr: Address, slot: H256) -> Result<H256, StateError>;
    fn code(&self, addr: Address, code_hash: H256) -> Result<Option<Vec<u8>>, StateError>;
}
```

Point reads. Used by the EVM (via `StoreVmDatabase`), RPC handlers, and any
read-only consumer.

### `StateCommitter` trait

```rust
pub trait StateCommitter: StateReader {
    fn update_accounts(&mut self, addrs: &[Address], muts: &[AccountMut])
        -> Result<(), StateError>;
    fn update_storage(&mut self, addr: Address, slots: &[(H256, H256)])
        -> Result<(), StateError>;
    fn hash(&mut self) -> H256;
    fn commit(self) -> Result<CommitOutput, StateError>;
}
```

Used for non-pipelined code paths (genesis, snap sync, tests). For the
pipelined block execution path, use the `Merkleizer` enum instead.

### `StateBackend` enum (in `ethrex-storage`)

```rust
// crates/storage/backend.rs (new file)

pub enum StateBackend {
    Mpt(MptBackend),
    // PR 2: Binary(BinaryBackend),
    // PR 3: Transition(TransitionBackend),
}
```

Implements `StateReader` and `StateCommitter` by delegating to the inner
backend via match.

### `Merkleizer` enum (in `ethrex-storage`)

```rust
// crates/storage/merkleizer.rs (new file)

pub enum Merkleizer {
    Mpt(MptMerkleizer),
    // PR 2: Binary(BinaryMerkleizer),
}

impl Merkleizer {
    pub fn feed_updates(&mut self, updates: Vec<AccountUpdate>) -> Result<(), StateError> {
        match self {
            Merkleizer::Mpt(m) => m.feed_updates(updates),
        }
    }
    pub fn finalize(self) -> Result<MerkleOutput, StateError> {
        match self {
            Merkleizer::Mpt(m) => m.finalize(),
        }
    }
}
```

This is NOT a trait -- it is an enum with inherent methods, consistent with
the enum-dispatch principle. Each backend's merkleizer (`MptMerkleizer`,
`BinaryMerkleizer`) has matching inherent methods.

### Genesis entry points on `StateBackend`

Computing the genesis state root and building the genesis block are
backend-dependent (each backend hashes its alloc differently). These are
pure functions of `(BackendKind, Genesis)` -- they do not read stored
state, so they are **associated functions**, not `&self` methods. This
keeps them callable from places that have a `Genesis` but no `Store`
(CLI `compute-state-root` subcommand, L2 deployer, startup banner).

```rust
impl StateBackend {
    pub fn compute_genesis_root(kind: BackendKind, genesis: &Genesis) -> H256 {
        match kind {
            BackendKind::Mpt => ethrex_trie::genesis_root(genesis),
            // PR 2: BackendKind::Binary => ethrex_binary_trie::genesis_root(genesis),
        }
    }

    pub fn compute_genesis_block(kind: BackendKind, genesis: &Genesis) -> Block {
        match kind {
            BackendKind::Mpt => ethrex_trie::genesis_block(genesis),
            // PR 2: BackendKind::Binary => ethrex_binary_trie::genesis_block(genesis),
        }
    }
}
```

Callers that cannot see `ethrex-storage` (e.g. `ethrex-config` tests,
`ethrex-trie`'s own tests) call the backend-specific free functions in
`ethrex-trie` (`genesis_root`, `genesis_block`) directly. Every other
caller goes through `StateBackend`.

### Factory method on `StateBackend`

```rust
impl Merkleizer {
    /// Create a streaming MPT merkleizer for pipelined block execution.
    /// Workers are spawned immediately on the shared `rayon::ThreadPool`.
    ///
    /// Does NOT take `&Store`. Instead takes an `Arc<dyn TrieProvider>` so
    /// that `ethrex-trie` does not depend on `ethrex-storage`. The provider
    /// knows how to open both state and storage tries.
    pub fn new_mpt(
        parent_state_root: H256,
        precompute_witnesses: bool,
        provider: Arc<dyn TrieProvider>,
        pool: Arc<rayon::ThreadPool>,
    ) -> Result<Self, StateError> { ... }
}
```

`ethrex-storage` constructs the opener closures from `Store` and passes them
in. This preserves the dependency inversion: `ethrex-trie` never imports
`Store`.

### Flat state iteration on `StateBackend`

```rust
impl StateBackend {
    /// Iterate accounts starting from `from` under the given `root`.
    /// Used by the flat state generator (background thread).
    /// Box<dyn Iterator> is acceptable -- not latency-sensitive.
    pub fn iter_accounts(
        &self,
        root: H256,
        from: Option<H256>,
    ) -> Result<Box<dyn Iterator<Item = (H256, Vec<u8>)> + '_>, StateError> {
        match self {
            StateBackend::Mpt(mpt) => mpt.iter_accounts(root, from),
        }
    }

    /// Iterate storage slots for an account.
    pub fn iter_storage(
        &self,
        root: H256,
        account: H256,
        storage_root: H256,
        from: Option<H256>,
    ) -> Result<Box<dyn Iterator<Item = (H256, Vec<u8>)> + '_>, StateError> {
        match self {
            StateBackend::Mpt(mpt) => mpt.iter_storage(root, account, storage_root, from),
        }
    }
}
```

The flat state generator holds an owned `StateBackend` (cloned from Store's
backend config, not borrowed). This avoids lifetime issues with the background
thread.

---

## 5. Flat State System

### Tables (trie-agnostic, shared by all backends)

| Table | Key | Value |
|-------|-----|-------|
| `ACCOUNT_FLAT_STATE` | Nibble path (account) | RLP-encoded value |
| `STORAGE_FLAT_STATE` | Prefixed nibble path (account + storage) | RLP-encoded value |

Currently named `ACCOUNT_FLATKEYVALUE` and `STORAGE_FLATKEYVALUE`. The names
can stay or be renamed -- the key point is they are shared, not duplicated per
backend. Both MPT and binary trie write the same format: `(hashed_key, encoded_value)`.

### Format marker

A single-byte discriminator is stored in `MISC_VALUES` under key
`state_backend_format` (constant `STATE_BACKEND_FORMAT_KEY` in `tables.rs`).
Byte `0` means MPT nibble-path format. `Store::from_backend` writes this marker
on the first open of a fresh database, and validates it on every subsequent
open. If the on-disk byte does not match the configured `BackendKind` the store
refuses to start with a `StoreError::Custom` describing the mismatch. This
prevents a database written by one backend from being silently read by another
with an incompatible key format. A new backend must claim a unique byte value
(documented next to `STATE_BACKEND_FORMAT_KEY`) and either reuse the MPT
nibble-path layout or supply a migration path.

### How reads work

`StateReader` implementations check flat state first, then fall back to trie
traversal. This is the current behavior via `BackendTrieDB.flatkeyvalue_computed()`
and the `TrieWrapper` layer.

### Flat state generator

Background thread spawned in `Store::from_backend`. Iterates the trie and
writes leaf key-value pairs to the flat tables for fast point reads.

The generator lives in the backend's wiring module (e.g. `mpt_wiring.rs`)
because it needs fine-grained DB transaction control: batch commits every
10k entries, resume from a `last_written` cursor, progress markers, and
stop/continue signals during disk flushes. A simple `Box<dyn Iterator>`
cannot provide this level of control.

**Each backend provides its own FKV generator.** The shared flat tables
(`ACCOUNT_FLATKEYVALUE`, `STORAGE_FLATKEYVALUE`) are the same, but the
iteration logic is backend-specific:
- **MPT** (`mpt_wiring::flatkeyvalue_generator`): opens `BackendTrieDB`,
  iterates `Node::Leaf`, decodes `AccountState` to find storage roots
- **Binary trie** (PR 2, `binary_wiring::flatkeyvalue_generator`): iterates
  stems, decodes binary leaf format

`Store::from_backend` dispatches to the correct generator based on the
active backend. When adding a new backend, you MUST implement a FKV
generator in its wiring module, otherwise flat state will not be populated
and reads will fall back to full trie traversal (slow).

The generator is paused during disk commits (receives `Stop`/`Continue`
control messages). This control flow is shared infrastructure in `store.rs`;
only the iteration source is backend-specific.

---

## 6. Merkleization

### Streaming design

1. Execution sends `Vec<AccountUpdate>` batches over a channel.
2. The merkleizer thread receives batches and calls `merkleizer.feed_updates()`.
3. When the channel drains, calls `merkleizer.finalize()` for state root + diffs.

```rust
// In execute_block_pipeline (blockchain.rs), the merkleizer thread becomes:
let provider = store.make_trie_provider(parent_state_root);
let merkleizer = Merkleizer::new_mpt(
    parent_state_root, precompute_witnesses, provider, merkle_pool,
)?;
let merkle_handle = std::thread::Builder::new()
    .name("block_executor_merkleizer".to_string())
    .spawn_scoped(s, move || {
        for updates in rx {
            merkleizer.feed_updates(updates)?;
        }
        merkleizer.finalize()
    })?;
```

The `Merkleizer` enum is `Send` (workers internally hold `Arc`-shared state).
The scoped thread takes ownership via `move`.

### MPT implementation: `MptMerkleizer`

Lives in `crates/common/trie/merkleizer.rs` (the `ethrex-trie` crate).
Extracts the current `handle_merkleization` + `handle_subtrie` logic from
`blockchain.rs`.

```rust
pub struct MptMerkleizer {
    workers_tx: Vec<cb::Sender<WorkerRequest>>,
    watcher_rx: Option<cb::Receiver<Option<StateError>>>,
    code_updates: Vec<(H256, Code)>,
    hashed_address_cache: FxHashMap<Address, H256>,
    has_storage: FxHashSet<H256>,
    /// When true, accumulates updates for witness pre-computation.
    accumulate_for_witness: bool,
    accumulator: Option<FxHashMap<Address, AccountUpdate>>,
    provider: Arc<dyn TrieProvider>,
    parent_state_root: H256,
    bal_all_updates: Option<FxHashMap<Address, AccountUpdate>>,
    pool: Arc<rayon::ThreadPool>,
}

impl MptMerkleizer {
    pub fn new(
        parent_state_root: H256,
        precompute_witnesses: bool,
        provider: Arc<dyn TrieProvider>,
        pool: Arc<rayon::ThreadPool>,
    ) -> Result<Self, StateError> { ... }

    pub fn feed_updates(&mut self, updates: Vec<AccountUpdate>) -> Result<(), StateError> { ... }

    pub fn finalize(self) -> Result<MerkleOutput, StateError> { ... }
}
```

Types that move from `blockchain.rs` into `merkleizer.rs`:
- `WorkerRequest` enum
- `handle_subtrie` function (becomes internal to workers)
- `CollectedStateMsg`, `PreMerkelizedAccountState`, `BalStateWorkItem`
- `collapse_root_node` helper
- `collect_trie` helper
- `DROP_SENDER` static (async BranchNode drop mechanism)

All `StoreError` usages in the moved code are converted to `StateError`.

### `TrieProvider` trait

Dep-inversion seam: `MptMerkleizer` and `MptBackend` need to open tries
from storage, but `ethrex-trie` cannot name `Store`. The `TrieProvider`
trait declares the minimum interface the MPT code needs; `ethrex-storage`
provides the implementation (`StoreTrieProvider`).

```rust
pub trait TrieProvider: Send + Sync {
    fn open_state_trie(&self, root: H256) -> Result<Trie, TrieError>;
    fn open_storage_trie(
        &self,
        account_hash: H256,
        storage_root: H256,
    ) -> Result<Trie, TrieError>;
}
```

Also provides a no-op `EmptyTrieProvider` inside `backend.rs` for genesis
and witness-construction paths where tries are pre-loaded and the provider
is never called.

### Nibbles -> Vec<u8> conversion

`MptMerkleizer::finalize` converts `(Nibbles, NodeRLP)` pairs to
`(nibbles.into_vec(), node_rlp)` before placing them into `NodeUpdates::Mpt`.
The `apply_trie_updates` function in `ethrex-storage` converts back to
`Nibbles` when inserting into the `TrieLayerCache` (which internally uses
`Nibbles` via `TrieWrapper`). This boundary conversion is explicit and
localized.

### BAL merkleization

The current codebase has two paths: `handle_merkleization` (standard) and
`handle_merkleization_bal` (BAL-optimized, blocks until execution completes
then does greedy bin-packing). Both move into `MptMerkleizer`. The BAL path
is selected via a constructor flag: `MptMerkleizer::new_bal(...)`.
BAL workers get trie access through the same `TrieProvider` abstraction.

### Witness pre-computation

The current `handle_merkleization` returns `Option<Vec<AccountUpdate>>` for
the witness path (when `precompute_witnesses` is enabled). `MptMerkleizer`
supports this via the `accumulate_for_witness` flag. When enabled,
`feed_updates` clones each batch into `accumulated_updates`. `finalize`
serializes them into `MerkleOutput.accumulated_updates`.

---

## 7. Storage Layer

### Table layout

**Flat state tables (shared, trie-agnostic):**

| Table | Current name | Purpose |
|-------|-------------|---------|
| Account flat state | `ACCOUNT_FLATKEYVALUE` | Leaf values for fast account reads |
| Storage flat state | `STORAGE_FLATKEYVALUE` | Leaf values for fast storage reads |

**Trie node tables (backend-specific):**

| Table | Current name | Backend |
|-------|-------------|---------|
| MPT account nodes | `ACCOUNT_TRIE_NODES` | MPT |
| MPT storage nodes | `STORAGE_TRIE_NODES` | MPT |
| Binary trie nodes | `BINARY_TRIE_NODES` (PR 2) | Binary |

### Trie layer cache

`TrieLayerCache` in `layering.rs` is a byte-level KV overlay with a bloom
filter. **This stays generic** -- it operates on raw bytes.

The `put_batch` signature is generalized from `Vec<(Nibbles, Vec<u8>)>` to
`Vec<(Vec<u8>, Vec<u8>)>`. Call sites that currently pass `Nibbles` will
call `nibbles.into_vec()` at the boundary.

Affected call sites for `put_batch` / `apply_prefix`:
- `store.rs` `apply_trie_updates` (~line 3283)
- `store.rs` `setup_genesis_state_trie` (~line 2125, 2131)
- `layering.rs` `TrieWrapper::put_batch` (line ~331)

`TrieWrapper` stays `Nibbles`-based internally (it implements `TrieDB`). The
conversion from `Vec<u8>` to `Nibbles` happens at the `TrieWrapper` boundary.

### Trie update worker

The `trie_update_worker` thread receives `TrieUpdate` messages. `TrieUpdate`
is changed to carry `NodeUpdates` (the enum) instead of raw types:

```rust
struct TrieUpdate {
    result_sender: SyncSender<Result<(), StoreError>>,
    parent_state_root: H256,
    child_state_root: H256,
    node_updates: NodeUpdates,
    code_updates: Vec<(H256, Code)>,
}
```

This replaces the current `account_updates: TrieNodesUpdate` and
`storage_updates: Vec<(H256, TrieNodesUpdate)>` fields.

### `UpdateBatch` reshape

`UpdateBatch` on main used `Vec<TrieNode>` for its `account_updates` /
`storage_updates` fields, where `TrieNode = (Nibbles, NodeRLP)` — MPT-typed.
Those two fields are replaced by a single `node_updates: NodeUpdates` enum
carrying the backend-specific diffs. The rest (`blocks`, `receipts`,
`code_updates`, `batch_mode`) is unchanged. `Store::store_block_updates`
still takes `UpdateBatch`; the pipeline produces it from the merkleizer's
`MerkleOutput`.

```rust
pub struct UpdateBatch {
    pub node_updates: NodeUpdates,
    pub code_updates: Vec<(H256, Code)>,
    pub blocks: Vec<Block>,
    pub receipts: Vec<(H256, Vec<Receipt>)>,
    pub batch_mode: bool,
}
```

`AccountUpdatesList` (separate type on main) is deleted outright — its role
is subsumed by `MerkleOutput`.

---

## 8. Breaking `ethrex-common` -> `ethrex-trie` Dependency

Currently `ethrex-common` depends on `ethrex-trie`. This must be broken so
that `ethrex-trie` can depend on `ethrex-common` (for `AccountState`,
`AccountUpdate` in `MptMerkleizer`) without a cycle.

### Consensus-level roots vs state roots

Transaction roots, receipt roots, and withdrawal roots are consensus-level
commitments defined by the block format. Even EIP-7864 only changes the state
trie -- these block-level roots remain MPT forever.

All `compute_*_root` functions (`compute_transactions_root`,
`compute_receipts_root`, `compute_withdrawals_root`, `compute_storage_root`,
`compute_state_root`) and `validate_block_body` **move to `ethrex-trie`**.
They all call `Trie::compute_hash_from_unsorted_iter` which requires the
`Trie` type.

Every caller of these functions already has `ethrex-trie` in its dependency
tree (through `ethrex-storage`, `ethrex-blockchain`, or transitively through
`ethrex-common` -> `ethrex-vm` today). After the dep inversion, callers
import from `ethrex-trie` directly.

### Guest program and zkVM

The guest program uses `MptBackend` directly (concrete type, no `StateBackend`
enum) since it knows at compile time which backend it needs. When binary trie
arrives (PR 2), the guest program swaps `MptBackend` for `BinaryBackend`. The
choice is compile-time (different guest binaries) or runtime (match on witness
format).

`MptBackend` gets a `from_witness` constructor that builds tries from execution
witness data. This is what `GuestProgramState::from_witness` currently does
internally.

`GuestProgramState` moves entirely to `ethrex-trie`. `ethrex-vm` adds
`ethrex-trie` as a dependency.

**No feature gating is needed.** The heavy deps (`crossbeam`, `rayon`) that
`ethrex-trie` pulls in already reach the guest program through other paths
(`ethrex-common` uses `rayon`, `ethrex-levm` uses `rayon`). These crates
compile for zkVM targets with no-op thread pool fallbacks. Feature gating
`ethrex-trie` would not reduce the dep footprint.

### Current usages and how to resolve them

| File | Usage | Resolution |
|------|-------|------------|
| `types/block.rs` | `Trie::compute_hash_from_unsorted_iter()` for tx/receipt/withdrawal roots | **Move** `compute_transactions_root`, `compute_receipts_root`, `compute_withdrawals_root`, and `validate_block_body` to `ethrex-trie`. All callers already have `ethrex-trie` in their dep tree. |
| `types/account.rs` | `Trie::compute_hash_from_unsorted_iter()` for `compute_storage_root()` | **Move** `compute_storage_root` to `ethrex-trie`. `From<GenesisAccount> for AccountState` defaults `storage_root` to `EMPTY_TRIE_HASH`; the real root is computed in the genesis setup path in `ethrex-storage`. |
| `types/genesis.rs` | `Trie::compute_hash_from_unsorted_iter()` for `compute_state_root()` | **Move** `compute_state_root` to `ethrex-trie`. |
| `types/genesis.rs` | `genesis_block()` method | **Move** to `ethrex-trie` as a free function `genesis_block(genesis: &Genesis)` in `genesis.rs`. Callers with a `BackendKind` use `StateBackend::compute_genesis_block` instead. |
| `types/block_execution_witness.rs` | `Node`/`NodeRef` trie data in `ExecutionWitness` struct | **Change shape**: `state_trie_root: Option<Node>` + `storage_trie_roots: BTreeMap<H256, Node>` collapse into `state_proof: Vec<Vec<u8>>`. Keeps `ExecutionWitness` and `RpcExecutionWitness` backend-agnostic in `ethrex-common`. Trie reconstruction moves into `MptBackend::from_witness_bytes`. |
| `types/block_execution_witness.rs` | `TryFrom<ExecutionWitness> for RpcExecutionWitness` | **Replace** with `impl From<ExecutionWitness> for RpcExecutionWitness` (infallible now that `state_proof` is already bytes). Reverse conversion becomes `RpcExecutionWitness::into_execution_witness(chain_config, first_block_number)` returning `Result<_, ExecutionWitnessConversionError>`. Both stay in `ethrex-common`. |
| `types/block_execution_witness.rs` | `GuestProgramState` + `GuestProgramStateError` | **Move** to `ethrex-vm` (`crates/vm/guest_program_state.rs`). It is a zkVM-facing type, not a common data struct. Consumers (`crates/vm/witness_db.rs`, `crates/guest-program/`) import from `ethrex_vm`. |
| `common.rs` | Re-exports `TrieLogger`, `TrieWitness` | **Remove** re-exports. Consumers import from `ethrex-trie` directly. |
| `rkyv_utils.rs` | Re-exports `H256Wrapper` | **Move** `H256Wrapper` definition to `ethrex-common` (it is a serialization wrapper, not trie-specific). |
| `rlp/benches/decode.rs` | `Nibbles`, `NodeHash` in benchmarks | Keep as dev-dependency. Cargo allows dev-dep cycles. Note: with `ethrex-trie` depending on `ethrex-common`, the `ethrex-rlp [dev] -> ethrex-trie -> ethrex-common -> ethrex-rlp` path is a dev-dep cycle, which Cargo permits. |

### L2 callers: `MerkleOutput` migration

`apply_account_updates_batch` changes its return type to `MerkleOutput`. L2
callers (`l1_committer.rs`, `block_producer.rs`) update to use
`MerkleOutput.root` and `MerkleOutput.node_updates`. This is a mechanical
change to files in `crates/l2/`.

### `EMPTY_TRIE_HASH` dual definition

`EMPTY_TRIE_HASH` exists in both `ethrex-trie::trie.rs` (lazy_static) and
`ethrex-common::constants.rs` (LazyLock). After breaking the dep:
- `ethrex-common::constants::EMPTY_TRIE_HASH` remains for `AccountState::default()`
  (it uses `keccak_hash` from `ethrex-crypto`, no trie dep needed).
- `ethrex-trie::EMPTY_TRIE_HASH` remains for trie internals.
- Both compute the same value. No conflict.
- `block_execution_witness.rs` trie methods (moved to `ethrex-trie`) use
  `crate::EMPTY_TRIE_HASH`.

---

## 9. PR Split

### PR 1 -- Complete abstraction with MPT only

**What ships:**
- Break `ethrex-common` -> `ethrex-trie` dependency (Section 8)
- Move all `compute_*_root` functions and `validate_block_body` to `ethrex-trie`
- Add `ethrex-common` as dependency of `ethrex-state-backend` and `ethrex-trie`
- `ethrex-vm` depends on `ethrex-trie` (for `MptBackend`)
- `ethrex-state-backend` crate with `StateReader`, `StateCommitter`, shared types
- `MptBackend` in `ethrex-trie` implementing `StateReader` + `StateCommitter`,
  with `from_witness_bytes` constructor
- `GuestProgramState` moved to `ethrex-vm` (zkVM-facing state)
- `compute_storage_root` and `compute_state_root` moved to `ethrex-trie`
- `genesis_block` / `genesis_root` moved to `ethrex-trie` as free functions
- `ExecutionWitness` + `RpcExecutionWitness` conversions stay in `ethrex-common`
  (`From`/`into_execution_witness`), now infallible
- `MptMerkleizer` in `ethrex-trie` (extracted from `handle_merkleization` / `handle_subtrie`)
- `StateBackend` enum in `ethrex-storage` with single `Mpt` arm
- `Merkleizer` enum in `ethrex-storage` with single `Mpt` arm
- `MerkleOutput` + `NodeUpdates::Mpt` replacing `AccountUpdatesList` and `UpdateBatch`
- `apply_account_updates_batch` returns `MerkleOutput`; L2 callers updated
- `TrieLayerCache::put_batch` generalized to `Vec<(Vec<u8>, Vec<u8>)>`
- `TrieUpdate` carries `NodeUpdates` instead of raw types
- `execute_block_pipeline` uses `StateBackend::new_merkleizer()`
- Flat state generator uses `StateBackend::iter_accounts/iter_storage`
- All blockchain/storage call sites go through `StateBackend`
- `block_execution_witness.rs` trie logic moved to `ethrex-trie`
- `TrieLogger`/`TrieWitness` re-exports removed from `ethrex-common`
- `StoreVmDatabase` cache changed from keccak-hashed to raw `Address`

**What does NOT ship:**
- No `Binary` variant
- No `Transition` variant
- No binary trie code
- No changes to snap sync / healing (they remain MPT-specific)

**Exit criteria:**
- [ ] `ethrex-common` does not depend on `ethrex-trie`
- [ ] `compute_transactions_root`, `compute_receipts_root`,
      `compute_withdrawals_root`, `validate_block_body` live in `ethrex-trie`
- [ ] `ethrex-state-backend` depends on `ethrex-common`
- [ ] `ethrex-trie` depends on `ethrex-common` and `ethrex-state-backend`
- [ ] `ethrex-vm` depends on `ethrex-trie`
- [ ] `GuestProgramState` lives in `ethrex-vm`
- [ ] `MptBackend::from_witness_bytes` exists and builds tries from witness data
- [ ] `ExecutionWitness`/`RpcExecutionWitness` conversions live in `ethrex-common`
- [ ] `compute_storage_root` and `compute_state_root` live in `ethrex-trie`
- [ ] `genesis_block` / `genesis_root` live in `ethrex-trie` as free functions
- [ ] `StateBackend` enum exists in `ethrex-storage` with `Mpt` arm
- [ ] `Merkleizer` enum exists in `ethrex-storage` with `Mpt` arm
- [ ] `MptMerkleizer` in `ethrex-trie` has `feed_updates` + `finalize`
- [ ] `blockchain.rs` no longer contains `handle_subtrie`, `WorkerRequest`,
      `handle_merkleization`, or direct `Trie` manipulation in the
      merkleization path
- [ ] `AccountUpdatesList` and `UpdateBatch` types deleted
- [ ] `apply_account_updates_batch` returns `MerkleOutput`
- [ ] L2 callers (`l1_committer.rs`, `block_producer.rs`) use `MerkleOutput.root`
      and `MerkleOutput.node_updates`
- [ ] `apply_account_updates_from_trie_batch` and
      `apply_account_updates_from_trie_with_witness` refactored through
      `StateBackend`
- [ ] Flat state generator uses `StateBackend::iter_accounts/iter_storage`
- [ ] No `ethrex_trie` imports in `ethrex-common`
- [ ] `make lint` passes
- [ ] `make test` passes
- [ ] Benchmark regression <= 2% (build_block_benchmark)

### PR 2 -- Add binary trie backend

- `BinaryBackend` implementing `StateReader`, `StateCommitter`
- `BinaryMerkleizer` with inherent `feed_updates` + `finalize`
- `StateBackend::Binary` and `Merkleizer::Binary` arms
- `NodeUpdates::Binary` variant
- `BINARY_TRIE_NODES` table
- Cross-backend conformance tests

### PR 3 -- Transition backend + fork activation

- `TransitionBackend { base: MptBackend, overlay: BinaryBackend }`
- Read path: overlay first, fall back to base
- Write path: overlay only
- Fork activation gate on timestamp / config
- Designed to be deletable once migration completes

---

## 10. Call Site Migration (PR 1)

### `crates/blockchain/blockchain.rs`

| Current code | Action |
|-------------|--------|
| `handle_merkleization` (line ~590) | Delete. Logic moves to `MptMerkleizer::feed_updates` + `finalize`. |
| `handle_merkleization_bal` (line ~820) | Delete. Moves to `MptMerkleizer::new_bal`. |
| `handle_subtrie` (line ~2683) | Delete. Internal to `MptMerkleizer` workers. |
| `WorkerRequest` enum (line ~261) | Delete. Moves to `merkleizer.rs`. |
| `CollectedStateMsg`, `PreMerkelizedAccountState`, `BalStateWorkItem` | Delete. Move to `merkleizer.rs`. |
| `collapse_root_node` helper (line ~2598) | Moves inside `MptMerkleizer`. |
| `DROP_SENDER` static (line ~127) | Moves to `merkleizer.rs` (async BranchNode drop). |
| `execute_block_pipeline` (line ~414) | Refactor: create `StateBackend`, construct openers from `Store`, get merkleizer, call `feed_updates`/`finalize`. |

### `crates/storage/store.rs`

| Current code | Action | Status |
|-------------|--------|--------|
| `AccountUpdatesList` | Delete. Replaced by `MerkleOutput`. | DONE |
| `UpdateBatch` | Reshape: `account_updates`/`storage_updates` MPT fields collapse into `node_updates: NodeUpdates`. `blocks`/`receipts`/`code_updates`/`batch_mode` stay. | DONE |
| `store_block_updates` | Accepts reshaped `UpdateBatch`. | DONE |
| `apply_account_updates_batch` | Routes through `StateBackend`, returns `MerkleOutput`. | DONE |
| `apply_account_updates_from_trie_batch` / `_with_witness` | Deleted; logic subsumed by `MptBackend::apply_witness_updates` and the `StateCommitter` commit path. | DONE |
| `TrieUpdate` struct | Change fields to `NodeUpdates` enum. | DONE |
| `apply_trie_updates` | Match on `NodeUpdates`. Convert back to `Nibbles`. | DONE |
| `flatkeyvalue_generator` | Remains MPT-specific (`mpt_wiring.rs`). Each backend provides its own. | DONE |
| `hash_address()`, `hash_key()` | Free functions in `ethrex-trie` (`crates/common/trie/backend.rs`), return `[u8; 32]`. | DONE |
| `get_account_info_by_state_root` / `get_nonce_by_account_address` / `get_code_by_account_address` | Delegate to `new_state_reader()` via `StateReader`. | DONE |
| `get_storage_at_root` | Kept as a direct low-level method (bypasses `StateBackend::storage`) to preserve main's FKV short-circuit when `flatkeyvalue_computed_with_last_written` is true. | DONE |
| `get_storage_at_root_with_known_storage_root` | Deleted. VM SLOAD path uses `StateReader::storage` via `StoreVmDatabase`; the root cache moved inside `MptBackend` (`storage_root_cache: Mutex<FxHashMap<H256, H256>>`). | DONE |

### `crates/storage/layering.rs`

| Current code | Action |
|-------------|--------|
| `TrieLayerCache::put_batch` (line ~175) | Generalize from `Vec<(Nibbles, Vec<u8>)>` to `Vec<(Vec<u8>, Vec<u8>)>`. |
| `TrieWrapper::put_batch` (line ~331) | Convert `Vec<u8>` back to `Nibbles` at this boundary. |
| `apply_prefix` | Keep as-is (operates on `Nibbles` within the MPT path). |

### `crates/common/types/`

| Current code | Action |
|-------------|--------|
| `block.rs` `compute_transactions_root`, `compute_receipts_root`, `compute_withdrawals_root` | **Move** to `ethrex-trie`. All callers have `ethrex-trie` in their dep tree. |
| `block.rs` `validate_block_body` | **Move** to `ethrex-trie` (calls `compute_*_root`). |
| `account.rs` `compute_storage_root` | **Move** to `ethrex-trie`. |
| `account.rs` `From<GenesisAccount> for AccountState` | **Keep** in `ethrex-common`. Default `storage_root` to `EMPTY_TRIE_HASH`. Real root computed in genesis setup path in `ethrex-storage`. |
| `genesis.rs` `compute_state_root` | **Move** to `ethrex-trie`. |
| `genesis.rs` `genesis_block()` | **Move** to `ethrex-trie` as free function `genesis_block`. |
| `block_execution_witness.rs` trie logic | Collapse `Node`/`NodeRef` fields into `state_proof: Vec<Vec<u8>>`. Trie reconstruction moves to `MptBackend::from_witness_bytes`. `ExecutionWitness` + `RpcExecutionWitness` stay in `ethrex-common`. |
| `block_execution_witness.rs` `TryFrom<ExecutionWitness> for RpcExecutionWitness` | Replace with `impl From<ExecutionWitness> for RpcExecutionWitness` (infallible). Reverse conversion: `RpcExecutionWitness::into_execution_witness(chain_config, first_block_number)`. Both stay in `ethrex-common`. |
| `common.rs` re-exports | Remove `TrieLogger`, `TrieWitness` re-exports. |

### `crates/blockchain/vm.rs`

| Current code | Action |
|-------------|--------|
| Line 102: `keccak_hash(address)` for VM cache key | Cache by raw `Address` (no pre-hash). |

### `crates/vm/`

| Current code | Action |
|-------------|--------|
| `db.rs` `VmDatabase` trait | Returns `AccountStateInfo` instead of `AccountState` (hides `storage_root`). |
| `guest_program_state.rs` (new) | Hosts `GuestProgramState` + `GuestProgramStateError`. Moved here from `ethrex-common`. |
| `witness_db.rs` | Imports `GuestProgramState` from `crate::` (same crate). |
| `Cargo.toml` | Adds `ethrex-trie`, `ethrex-state-backend`, `hex`. |

### `crates/guest-program/`

| Current code | Action |
|-------------|--------|
| `GuestProgramState` usage | Import from `ethrex_vm::GuestProgramState`. |
| State construction from witness | `GuestProgramState::from_witness` wraps `MptBackend::from_witness_bytes`. |

### `crates/l2/`

| Current code | Action |
|-------------|--------|
| `l1_committer.rs` use of `apply_account_updates_batch` | Update to use `MerkleOutput.root` and `MerkleOutput.node_updates` from the new return type. |
| `block_producer.rs` use of `apply_account_updates_batch` | Update to use `MerkleOutput.root` and `MerkleOutput.node_updates`. |

### Files NOT modified

- `crates/vm/db.rs` -- `VmDatabase` trait stays as-is
- `crates/networking/` -- snap sync / healing stay MPT-specific (import
  `EMPTY_TRIE_HASH` from `ethrex-trie` directly, already correct)

---

## 11. What Stays Backend-Specific

Two categories: **internal types** that must not leak into shared APIs, and
**capabilities** that every backend must support but with different internals.

### Backend-internal types (MUST NOT leak)

These are implementation details of each backend. They never appear in
`StateReader`, `StateCommitter`, or `StateBackend`'s public API:

**MPT-internal:**
- `Nibbles`, `NodeRef`, `BranchNode`, `TrieNode` types
- RLP encoding/decoding of account state
- `storage_root` per account (inside `AccountState`)
- keccak-of-address / keccak-of-slot key derivation
- 16-shard parallelism (internal to `MptMerkleizer`)

**Binary trie-internal (PR 2):**
- Stem layer cache, stem writes, code chunking
- BLAKE3 / binary merkle hashing
- 32-byte flat key derivation
- `BasicData` leaf encoding

### Capabilities every backend must support

These operations are NOT "MPT-only" -- every backend needs them, but with
different internal formats. When adding a new backend, the compiler forces
exhaustive handling via the `StateBackend` enum:

| Capability | Interface | Backend-specific internals |
|-----------|-----------|---------------------------|
| **State reads** | Trait: `StateReader::account`, `storage`, `code` | Key derivation, encoding |
| **State writes** | Trait: `StateCommitter::update_accounts`, `update_storage`, `commit` | Key derivation, trie structure |
| **Merkleization** | Enum: `Merkleizer::feed_updates`, `finalize` | Parallelism, node format |
| **Genesis** | Trait: `StateCommitter::update_accounts` + `commit` | Same as writes |
| **Proofs** | Trait: `StateReader::account_proof` (returns `Vec<Vec<u8>>`) | MPT: RLP nodes; Binary: stem proof nodes |
| **Witness output** | Trait method (returns serialized bytes) | MPT: RLP node list; Binary: stem witness |
| **Witness construction** | Enum match (needs backend-specific types during construction) | MPT: `TrieLogger`; Binary: stem recorder |
| **Sync protocol** | Enum match (protocol messages are backend-specific) | MPT: snap sync; Binary: stem sync |
| **Iteration** | Enum: `StateBackend::iter_accounts`, `iter_storage` | Trie traversal internals |

**Rule:** No capability is "forever MPT-only." If an operation exists for MPT,
plan for the binary trie to need it too. Either abstract it on the shared
trait, or put it on the `StateBackend` enum with exhaustive match.

### When to use trait methods vs enum match

**Prefer trait methods** when the operation can be described with a common
signature. Most capabilities fall into this category -- the output at the
boundary is typically `Vec<u8>`, `H256`, or a shared struct regardless of
backend internals:

```rust
// Proofs: both backends produce serialized bytes at the boundary
trait StateReader {
    fn account_proof(&self, addr: Address, keys: &[H256]) -> Result<Vec<Vec<u8>>, StateError>;
}
// MPT: returns RLP-encoded nodes. Binary: returns stem proof nodes.
// Caller gets Vec<Vec<u8>> either way.
```

**Use enum match only** when the caller genuinely needs backend-specific
internals (e.g. constructing a witness requires `TrieLogger` node-level
access, which is an MPT-specific type that has no binary trie equivalent):

```rust
// Witness construction: needs backend-specific types during construction
match &state_backend {
    StateBackend::Mpt(mpt) => {
        let (logger, trie) = TrieLogger::open_trie(mpt.open_trie(root)?);
        // ... walk trie nodes via TrieLogger, MPT-specific construction
    }
    // PR 2:
    // StateBackend::Binary(bin) => {
    //     // ... stem-based witness construction
    // }
}
```

**Rule of thumb:** if the output goes over the wire or into storage as bytes,
a trait method works. If the caller needs to interact with backend-specific
types *during the operation*, use enum match.

### Guest program uses concrete backend type

The guest program (and `ethrex-vm`) use `MptBackend` directly -- NOT the
`StateBackend` enum. The guest program knows at compile time which backend it
needs and should not pay for enum dispatch. When binary trie arrives (PR 2),
the guest program swaps `MptBackend` for `BinaryBackend` at the type level.

---

## 12. Hash Function Modularity

Different trie backends use different hash functions:
- **MPT**: keccak256 for everything (address hashing, slot hashing, node
  hashing, empty trie root).
- **Binary trie (EIP-7864)**: BLAKE3 or a custom binary merkle hash for node
  commitment; may use different key derivation for address/slot mapping.

### What each backend owns

The hash function is **not** on a shared trait. Each backend uses its own hash
internally. The shared types (`Account`, `MerkleOutput`, `H256`) are
hash-agnostic -- they carry 32-byte values regardless of which function
produced them.

| Concern | Where it lives | Backend-specific? |
|---------|---------------|-------------------|
| Address -> trie key | Inside `MptBackend::hashed()` / `BinaryBackend::derive_key()` | Yes |
| Slot -> trie key | Inside `MptBackend::hashed_slot()` / `BinaryBackend::derive_key()` | Yes |
| Node hashing (merkle root) | Inside `Trie::hash()` / `BinaryTrie::state_root()` | Yes |
| Empty trie root | `EMPTY_TRIE_HASH` (MPT) / `EMPTY_BINARY_ROOT` (binary) | Yes |
| Empty code hash | `EMPTY_KECCACK_HASH` = `keccak("")` | No -- Ethereum consensus constant, shared |

### Where keccak currently leaks (to fix in PR 1)

**Write path (DONE):**

| Location | Leak | Fix |
|----------|------|-----|
| `blockchain.rs`: `handle_merkleization`, `handle_subtrie` | keccak for address/slot hashing in merkleization workers | Moved into `MptMerkleizer` (Phase 5) |
| `blockchain/vm.rs`: `keccak_hash(address)` for VM cache key | Pre-hashes address with keccak for cache lookups | Changed to cache by raw `Address` (Task 7.11) |

**Read path (to fix):**

| Location | Leak | Fix |
|----------|------|-----|
| `store.rs`: `hash_address()`, `hash_key()` public functions | Free functions using keccak, called by 15+ Store methods for state reads | Make `pub(crate)`. Route external reads through `StateBackend` (see Section 14) |
| `store.rs`: `get_account_info_by_state_root`, `get_storage_at_root`, etc. | Directly hash addresses/slots with keccak, open raw tries | Delegate to `Store::new_state_reader()` which returns a `StateBackend` |
| `blockchain.rs`: witness generation imports `hash_address`, `hash_key` | Keccak for trie path derivation in witness code | Import from `ethrex-trie` as `hash_address`/`hash_key` (MPT-specific witness code matches on enum arm) |
| `blockchain/vm.rs`: `get_storage_slot` | Uses `keccak_hash(address)` for `get_storage_at_root_with_known_storage_root` | Use `MptBackend::storage_with_hint` via enum match (see Section 14) |
| `ethrex-common/constants.rs`: `EMPTY_TRIE_HASH` | Defined locally using `keccak_hash` | Keep -- `AccountState` is an MPT-encoded type. Binary trie defines its own empty root |

### Design rule

**No shared code path may assume keccak.** Address/slot hashing, node
hashing, and empty root constants are backend internals. The only
Ethereum-consensus hash constant that is truly shared is `EMPTY_KECCACK_HASH`
(keccak of empty bytes for empty code), which is part of the Ethereum
protocol, not a trie concern.

---

## 13. Maintenance Rules

1. **Minimalism over abstraction.** Add a trait method only when a concrete
   caller needs to dispatch across two implementations.

2. **MPT is canonical.** Do not regress MPT to accommodate a hypothetical
   future.

3. **Traits must be impl-neutral.** No `Nibbles`, `RLP`, `storage_trie` in
   traits. Equally no `stem`, `BasicData`, `BLAKE3`.

4. **`storage_root` never appears in new code outside MPT internals.**

5. **Transition module is isolated.** Own tests, single entry point. Designed
   to be deletable.

6. **Cross-backend conformance tests in CI (PR 2+).** Same mutation sequence
   produces equivalent reads on both backends.

7. **Never add backend-specific methods to the core traits.** Put them on the
   concrete backend type; callers match on the enum.

8. **Benchmark MPT-only throughput before/after PR 1.** Regression > 2% is a
   blocker.

9. **PR 1 must justify itself on MPT alone** -- cleaner separation, merkleization
   encapsulated, better testability.

10. **Extensions as inherent methods per enum arm.** Do not force MPT to stub
    binary-trie-only methods.

11. **No capability is "forever MPT-only."** If an operation exists for MPT
    (witness, proofs, genesis, sync), plan for the binary trie to need it too.
    Either abstract it on the shared trait or put it on the `StateBackend`
    enum with exhaustive match.

---

## 14. State Read Path

### Problem

`Store` has ~15 methods that directly open MPT tries and call
`hash_address(keccak(...))` / `hash_key(keccak(...))` to derive trie keys.
These hardcode keccak key derivation. If a binary trie backend is added,
every one of these methods breaks.

The write path (merkleization) is already routed through `Merkleizer` which
internalizes the hash function. The read path must follow the same pattern.

### Design: `Store::new_state_reader`

`Store` gets a factory method that returns a `StateBackend` rooted at a
given state root. All general-purpose state reads go through `StateReader`
trait methods. The backend handles key derivation internally.

```rust
impl Store {
    /// Create a StateBackend rooted at the given state root.
    /// Uses DB-backed tries and the code table for reads.
    pub fn new_state_reader(&self, state_root: H256) -> Result<StateBackend, StoreError> {
        let state_trie = self.open_state_trie(state_root)?;
        let provider = self.make_trie_provider(state_root);
        let code_reader = self.make_code_reader();
        Ok(StateBackend::new_mpt_with_db(
            state_trie,
            Arc::new(NativeCrypto),
            provider,
            code_reader,
        ))
    }
}
```

### Changes to `MptBackend`

`MptBackend` always holds a `TrieProvider` and a `CodeReader`. Genesis /
witness constructors pass `EmptyTrieProvider` and a no-op code reader so the
fields are always present (no `Option` indirection).

```rust
pub struct MptBackend {
    state_trie: Trie,
    storage_tries: BTreeMap<H256, Trie>,
    codes: BTreeMap<H256, Code>,
    crypto: Arc<dyn Crypto>,
    storage_opener: Arc<dyn TrieProvider>,
    code_reader: CodeReader,
    storage_root_cache: Mutex<FxHashMap<H256, H256>>,
    witness_state: Option<MptWitnessState>,
}
```

`StateReader` implementation checks pre-loaded data first (witness path),
then falls back to DB-backed accessors:

- `account()`: hashes address internally, reads from state trie, decodes
- `storage()`: hashes address, reads account to get storage_root, opens
  storage trie via `storage_opener`, hashes slot, reads value
- `code()`: checks in-memory map first, falls back to `code_reader`

### Inherent methods on `MptBackend` (not on trait)

Some callers need MPT-specific data that the shared trait does not expose:

```rust
impl MptBackend {
    /// Return full AccountState including storage_root.
    /// MPT-specific: binary trie has no per-account storage root.
    pub fn account_state(&self, addr: Address) -> Result<Option<AccountState>, StateError>;

    /// Read storage when the account's storage_root is already known.
    /// Avoids re-reading the account from the state trie.
    /// Performance optimization for the VM hot path (SLOAD).
    pub fn storage_with_hint(
        &self,
        hashed_addr: H256,
        storage_root: H256,
        slot: H256,
    ) -> Result<H256, StateError>;
}
```

Callers that need these methods match on the `StateBackend` enum:
```rust
match &state_backend {
    StateBackend::Mpt(mpt) => mpt.account_state(addr),
    // PR 2: StateBackend::Binary(bin) => bin.account_data(addr),
}
```

### Store method migration

| Method | Action |
|--------|--------|
| `get_account_info_by_state_root(root, addr)` | Delegate to `new_state_reader(root)?.account(addr)` |
| `get_nonce_by_account_address(block_number, addr)` | Build reader from block's state root, call `account(addr)` |
| `get_code_by_account_address(block_number, addr)` | Build reader, call `account` then `code` |
| `get_storage_at_root(root, addr, slot)` | Delegate to `new_state_reader(root)?.storage(addr, slot)` |
| `get_storage_at_root_with_known_storage_root(...)` | **Delete.** Callers use `StateReader::storage` or `MptBackend::storage_with_hint` |
| `hash_address()`, `hash_key()` | Make `pub(crate)` in store.rs. Add `hash_address`, `hash_key` to `ethrex-trie` for MPT-specific callers |
| `apply_account_updates_from_trie_batch` | Keep as internal MPT helper (`pub(crate)`). Takes raw `Trie`, stays MPT-internal |
| `apply_account_updates_from_trie_with_witness` | Same -- internal MPT helper |
| `setup_genesis_state_trie` | Keep. Could use `StateCommitter` but needs node-level DB writes for initial cache. Revisit in PR 2 |

### VM (`StoreVmDatabase`) changes

`StoreVmDatabase` gains a `state_backend: StateBackend` field constructed
once in `StoreVmDatabase::new`. Account reads use `MptBackend::account_state`
(via enum match). Storage reads use `MptBackend::storage_with_hint` (via
enum match) to preserve the performance optimization of skipping account
re-reads for every SLOAD.

### Witness generation (blockchain.rs) changes

Witness code is backend-specific (operates on `TrieLogger`, `Node`,
`NodeRef`). It matches on the `StateBackend` enum arm. The `hash_address` /
`hash_key` calls import from `ethrex_trie` (`hash_address`,
`hash_key`) instead of `ethrex_storage`. When binary trie is added in
PR 2, the witness match arm calls binary-trie-specific witness methods.

### Acceptance criteria

- [ ] No `hash_address` or `hash_key` in `ethrex-storage` public API
- [ ] No `keccak_hash` calls in `blockchain.rs` or `vm.rs`
- [ ] All state reads in `store.rs` that external callers use go through
      `StateReader` (via `new_state_reader`)
- [ ] `get_storage_at_root_with_known_storage_root` deleted
- [ ] `StoreVmDatabase` uses `StateBackend` for reads
- [ ] Witness code uses `hash_address`/`hash_key` from `ethrex-trie`
