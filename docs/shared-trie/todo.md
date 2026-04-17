# Shared Trie -- Friction Points Audit

Last updated after completing all planned work.

## Resolved

### 1. vm.rs -- direct StateBackend::Mpt(mpt) downcasting
**Status:** Fixed. StoreVmDatabase uses `StateBackend::account_state_info()`
and `StateReader::storage()`. Zero MPT downcasts. SLOAD cache internalized
in MptBackend.

### 2. apply_account_updates_with_witness signature
**Status:** Fixed. Old method (taking `Trie` + `MptStorageTries`) replaced
with StateBackend witness API: `init_witness`, `record_witness_accesses`,
`apply_updates_with_witness_state`, `advance_witness_to`, `finalize_witness`.
No MPT types in public signatures.

### 3. blockchain.rs -- witness generation (500+ lines of MPT internals)
**Status:** Fixed. All MPT code moved into MptBackend methods.
blockchain.rs has zero imports of Node, NodeRef, Trie, TrieLogger,
mpt_hash_address, mpt_hash_key.

### 4. VmDatabase returns AccountState
**Status:** Fixed. Returns `AccountStateInfo` (AccountInfo + has_storage).
`storage_root` does not leak into VM layer.

### 5. mpt_hash_address / mpt_hash_key in blockchain.rs
**Status:** Fixed. Zero occurrences.

### 6. AccountProof uses AccountState
**Status:** Fixed. Uses `AccountInfo` + `storage_root: H256`.

### 7. GuestProgramState bypasses abstraction
**Status:** Fixed. Holds `MptBackend` instead of raw Trie fields.
`hash_address`/`hash_key` free functions removed.

## Remaining (acceptable, isolated in MPT-specific code)

### has_state_root in mpt_wiring.rs
`Store::has_state_root` decodes MPT `Node` directly. Already isolated in
`mpt_wiring.rs`. When binary trie is added, add a parallel check in
`binary_wiring.rs` and dispatch based on backend config. Not worth
abstracting now since `Store` doesn't hold a backend type enum.

### FKV generator (mpt_wiring.rs)
Each backend provides its own FKV generator. Not abstracted.

### Snap sync (p2p)
MPT snap sync is protocol-level. Binary trie adds its own sync. Not abstracted.

## Missing traits (no longer needed)

| Originally planned | Resolution |
|-------------------|------------|
| `StateReader::hash_address()` | Not needed. vm.js and blockchain.rs no longer call it. |
| `WitnessGenerator` trait | Solved via StateBackend witness methods. |
| `StateBackend::has_state_root()` | Kept on Store in mpt_wiring.rs (see above). |
| `storage_with_hint` without storage_root | Removed from public API. SLOAD uses StateReader::storage(). |
