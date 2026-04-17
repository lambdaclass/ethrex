# Shared Trie Abstraction -- Remaining Work

PR 1 (shared-trie branch) is complete. This file tracks follow-up work.

## Completed tasks

### 1. Port GuestProgramState to MptBackend

**Status:** Done

GuestProgramState now holds `MptBackend` instead of raw `Trie` fields.
`hash_address`/`hash_key` free functions removed. Methods use backend
internals via `pub(crate)` fields. `flush_storage_roots` and
`hash_no_commit_state` added to `MptBackend`.

### 2. Wire genesis through StateCommitter

**Status:** Done

`setup_genesis_state_trie` uses `StateCommitter::update_accounts` +
`update_storage` + `commit()`. Node updates written via
`write_node_updates_direct`. Genesis state root verified via existing
`debug_assert_eq!`. 8675 EF tests pass.

### 3. Witness abstraction

**Status:** Done (different approach than originally planned)

Instead of a `WitnessContext` enum, witness generation was fully
extracted into `StateBackend` methods: `init_witness`,
`record_witness_accesses`, `apply_updates_with_witness_state`,
`advance_witness_to`, `finalize_witness`. `MptStorageTries` and `Trie`
no longer appear in any `StateBackend` public signature. `blockchain.rs`
has zero imports of `Node`, `NodeRef`, `TrieLogger`, `mpt_hash_address`,
`mpt_hash_key`.

`ExecutionWitness` is now backend-agnostic: `state_proof: Vec<Vec<u8>>`
replaces `state_trie_root: Option<Node>` and
`storage_trie_roots: BTreeMap<H256, Node>`.

### 4. Replace AccountProof.account with backend-agnostic type

**Status:** Done

`AccountProof` now holds `info: AccountInfo` + `storage_root: H256`
instead of `account: AccountState`.

### 5. VM layer cleanup

**Status:** Done (additional work beyond original plan)

- `VmDatabase` and LEVM `Database` traits return `AccountStateInfo`
  (no `storage_root`) instead of `AccountState`
- `StoreVmDatabase` has zero `StateBackend::Mpt` downcasts
- SLOAD cache internalized in `MptBackend` via `Mutex<FxHashMap>`
- `MptBackend` two-mode design removed (no `Option` fields)
- `storage_with_state_hint` removed from `StateBackend` public API

## Remaining tasks (PR 2+)

### 6. Make FKV generator backend-agnostic (optional)

**Priority:** Low (binary trie can have its own generator)
**Files:** `crates/storage/mpt_wiring.rs`, `crates/storage/store.rs`

Each backend provides its own generator in its wiring module.
Current design is documented in `adding-a-backend.md`.

### 7. Snap sync abstraction

**Priority:** Low (only needed when binary trie lands)
**Files:** `crates/networking/p2p/sync/`

MPT snap sync code is correctly isolated in `mpt_wiring.rs`.
A binary trie would add a parallel sync implementation.
See `adding-a-backend.md` for guidance.
