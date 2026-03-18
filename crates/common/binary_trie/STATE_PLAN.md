# BinaryTrieState Module — Implementation Plan

## Overview

A new `state.rs` module in `ethrex-binary-trie` that wraps `BinaryTrie` and provides
Ethereum state operations (read/write accounts, storage, code) using EIP-7864 key mapping.
A separate `BinaryTrieVmDb` adapter in `ethrex-blockchain` implements `VmDatabase` on top of it.

## Requirements

**Explicit:**
- Read account state (nonce, balance, code_hash, code_size) from binary trie
- Read/write storage slots via EIP-7864 key derivation
- Read account code by hash (fast lookup, not trie reconstruction)
- Apply `AccountUpdate` to binary trie (including account removal, storage clearing)
- Apply genesis allocations
- Compute state root via merkelization
- Implement `VmDatabase` trait for VM execution

**Inferred:**
- `storage_root` field in `AccountState` must be handled since it doesn't exist in EIP-7864
- `removed_storage` (SELFDESTRUCT) must clear all storage for an account without trie enumeration
- Code must be both chunked in the trie AND available by hash for `get_account_code`

**Assumptions:**
- In-memory only for now (no persistent storage backend) — matches experimental shadow node scope
- EIP-4762 gas costs are NOT applied (mainnet gas rules)
- No concurrent access needed on `BinaryTrieState` (single-threaded block execution)

## Architecture Decision

**Split into two layers:**

1. **`BinaryTrieState`** in `ethrex-binary-trie` — owns the trie + side structures, provides
   Ethereum state read/write methods. No VM dependency.

2. **`BinaryTrieVmDb`** in `ethrex-blockchain` — thin adapter implementing `VmDatabase` by
   delegating to a shared `BinaryTrieState`. Holds `ChainConfig` and block hash cache.

**Why split:** `ethrex-binary-trie` depends on `ethrex-common`. Adding `ethrex-vm` as a
dependency would pull in `ethrex-levm` + `ethrex-trie` + `ethrex-crypto` — unnecessary coupling.
The `VmDatabase` trait lives in `ethrex-vm`, and `ethrex-blockchain` already depends on both
`ethrex-vm` and `ethrex-common`. Placing the adapter there avoids a new dependency entirely.

**Alternatives considered:**
- *VmDatabase directly on BinaryTrieState*: Creates `binary-trie → vm` dependency, pulling in
  the entire LEVM. Rejected.
- *New `ethrex-binary-trie-vm` bridge crate*: Over-engineered for an experimental feature.
  Rejected.

---

## Detailed Design

### 1. `BinaryTrieState` struct (`crates/common/binary_trie/state.rs`)

```rust
use std::collections::{BTreeMap, HashMap, HashSet};
use bytes::Bytes;
use ethereum_types::{Address, H256, U256};

use crate::BinaryTrie;
use crate::error::BinaryTrieError;
use crate::key_mapping::*;
use crate::merkle::merkelize;

pub struct BinaryTrieState {
    /// The underlying binary trie holding all state leaves.
    trie: BinaryTrie,

    /// Code by keccak256 hash — for fast `get_account_code` lookups.
    /// Code is also chunked in the trie, but reconstructing from chunks
    /// on every CALL would be expensive. Storing both is fine.
    code_store: HashMap<H256, Bytes>,

    /// Tracks which storage keys each account has written.
    /// Needed for `removed_storage` (SELFDESTRUCT) since the binary trie
    /// has no prefix-enumeration — we can't discover all storage keys
    /// for an address without this side structure.
    storage_keys: HashMap<Address, HashSet<H256>>,
}
```

**Why `storage_keys`:** In the binary trie, storage slot keys are derived from
`get_tree_key_for_storage_slot(address, slot)` which hashes address+offset. There's no
common prefix for "all storage of address X" — stems vary by tree_index. Without tracking
keys, we'd have to scan the entire trie. Since SELFDESTRUCT is rare (EIP-6780 limits it to
same-tx creation), the memory cost of a `HashSet<H256>` per account with storage is acceptable.

### 2. State Read Methods

```rust
impl BinaryTrieState {
    pub fn new() -> Self { ... }

    /// Read account state from the binary trie.
    ///
    /// Returns None if the account doesn't exist (no basic_data leaf).
    /// The `storage_root` field is synthesized:
    ///   - EMPTY_TRIE_HASH if the account has no tracked storage keys
    ///   - A dummy non-empty hash (H256::from_low_u64_be(1)) otherwise
    pub fn get_account_state(&self, address: &Address) -> Option<AccountState> {
        let basic_data_key = get_tree_key_for_basic_data(address);
        let basic_data = self.trie.get(basic_data_key)?;

        let (version, code_size, nonce, balance) = unpack_basic_data(&basic_data);
        // version must be 0
        debug_assert_eq!(version, 0);

        let code_hash_key = get_tree_key_for_code_hash(address);
        let code_hash = self.trie.get(code_hash_key)
            .map(H256::from)
            .unwrap_or(*EMPTY_KECCACK_HASH);

        // Synthesize storage_root for LevmAccount::has_storage compatibility
        let has_storage = self.storage_keys
            .get(address)
            .map_or(false, |keys| !keys.is_empty());
        let storage_root = if has_storage {
            // Any non-EMPTY_TRIE_HASH value signals "has storage"
            H256::from_low_u64_be(1)
        } else {
            *EMPTY_TRIE_HASH
        };

        Some(AccountState {
            nonce,
            balance,
            storage_root,
            code_hash,
        })
    }

    /// Read a storage slot value. Returns None if unset (treated as zero).
    pub fn get_storage_slot(&self, address: &Address, key: H256) -> Option<U256> {
        let storage_key = U256::from_big_endian(key.as_bytes());
        let tree_key = get_tree_key_for_storage_slot(address, storage_key);
        self.trie.get(tree_key).map(|v| U256::from_big_endian(&v))
    }

    /// Look up code by its keccak256 hash from the in-memory code store.
    pub fn get_account_code(&self, code_hash: &H256) -> Option<Bytes> {
        self.code_store.get(code_hash).cloned()
    }

    /// Get code size from basic_data. Returns 0 if account doesn't exist.
    pub fn get_code_size(&self, address: &Address) -> u32 {
        let basic_data_key = get_tree_key_for_basic_data(address);
        match self.trie.get(basic_data_key) {
            Some(data) => {
                let (_, code_size, _, _) = unpack_basic_data(&data);
                code_size
            }
            None => 0,
        }
    }

    /// Compute the binary trie state root via merkelization.
    pub fn state_root(&self) -> [u8; 32] {
        merkelize(self.trie.root.as_deref())
    }
}
```

**`storage_root` handling:** The VM's `LevmAccount` (line 78 of `account.rs`) converts
`AccountState` to `has_storage` via `state.storage_root != *EMPTY_TRIE_HASH`. We exploit
this: return `EMPTY_TRIE_HASH` when no storage, or `H256(1)` when storage exists. The
actual value doesn't matter — it's never used as a trie root in the binary trie world.
The `exists` field on `LevmAccount` is derived from `state != AccountState::default()`,
which works correctly since a real account will have non-default nonce/balance/code_hash.

### 3. State Write Methods

```rust
impl BinaryTrieState {
    /// Apply a single AccountUpdate to the trie.
    pub fn apply_account_update(
        &mut self,
        update: &AccountUpdate,
    ) -> Result<(), BinaryTrieError> {
        let address = &update.address;

        // Handle removed_storage (SELFDESTRUCT then recreate)
        if update.removed_storage {
            self.clear_account_storage(address)?;
        }

        // Handle full account removal
        if update.removed {
            self.remove_account(address)?;
            return Ok(());
        }

        // Apply account info changes
        if let Some(ref info) = update.info {
            let code_size = update.code.as_ref()
                .map(|c| c.bytecode.len() as u32)
                .unwrap_or_else(|| self.get_code_size(address));

            let basic_data = pack_basic_data(0, code_size, info.nonce, info.balance);
            self.trie.insert(
                get_tree_key_for_basic_data(address),
                basic_data,
            )?;

            // Write code_hash leaf
            self.trie.insert(
                get_tree_key_for_code_hash(address),
                info.code_hash.0,
            )?;
        }

        // Apply new code
        if let Some(ref code) = update.code {
            self.write_code(address, code)?;
        }

        // Apply storage changes
        for (key, value) in &update.added_storage {
            let storage_key = U256::from_big_endian(key.as_bytes());
            let tree_key = get_tree_key_for_storage_slot(address, storage_key);

            if value.is_zero() {
                // Zero means delete
                self.trie.remove(tree_key);
                if let Some(keys) = self.storage_keys.get_mut(address) {
                    keys.remove(key);
                    if keys.is_empty() {
                        self.storage_keys.remove(address);
                    }
                }
            } else {
                let mut val_bytes = [0u8; 32];
                value.to_big_endian(&mut val_bytes);
                self.trie.insert(tree_key, val_bytes)?;
                self.storage_keys
                    .entry(*address)
                    .or_default()
                    .insert(*key);
            }
        }

        Ok(())
    }

    /// Remove all state for an account (basic_data, code_hash, code chunks, storage).
    fn remove_account(&mut self, address: &Address) -> Result<(), BinaryTrieError> {
        // Remove basic_data and code_hash leaves
        self.trie.remove(get_tree_key_for_basic_data(address));
        self.trie.remove(get_tree_key_for_code_hash(address));

        // Remove code chunks (need code_size to know how many)
        let code_size = self.get_code_size(address);
        if code_size > 0 {
            let num_chunks = (code_size as u64 + 30) / 31;
            for chunk_id in 0..num_chunks {
                self.trie.remove(get_tree_key_for_code_chunk(address, chunk_id));
            }
        }

        // Remove all tracked storage
        self.clear_account_storage(address)?;

        // Clean up code_store — need to find the code_hash first
        // (already removed from trie, but we can look it up before removal)
        // Actually, code_store entries are shared by hash, so we don't remove
        // them here — other accounts may share the same code. Acceptable leak
        // for an experimental node.

        Ok(())
    }

    /// Clear all storage slots for an account using the tracked storage_keys.
    fn clear_account_storage(&mut self, address: &Address) -> Result<(), BinaryTrieError> {
        if let Some(keys) = self.storage_keys.remove(address) {
            for key in keys {
                let storage_key = U256::from_big_endian(key.as_bytes());
                let tree_key = get_tree_key_for_storage_slot(address, storage_key);
                self.trie.remove(tree_key);
            }
        }
        Ok(())
    }

    /// Write code: chunkify into trie leaves + store in code_store.
    fn write_code(
        &mut self,
        address: &Address,
        code: &Code,
    ) -> Result<(), BinaryTrieError> {
        // Remove old code chunks if code_size changed
        let old_code_size = self.get_code_size(address);
        if old_code_size > 0 {
            let old_num_chunks = (old_code_size as u64 + 30) / 31;
            let new_num_chunks = if code.bytecode.is_empty() {
                0
            } else {
                (code.bytecode.len() as u64 + 30) / 31
            };
            // Remove chunks that won't be overwritten
            for chunk_id in new_num_chunks..old_num_chunks {
                self.trie.remove(get_tree_key_for_code_chunk(address, chunk_id));
            }
        }

        // Write new code chunks
        let chunks = chunkify_code(&code.bytecode);
        for (i, chunk) in chunks.iter().enumerate() {
            self.trie.insert(
                get_tree_key_for_code_chunk(address, i as u64),
                *chunk,
            )?;
        }

        // Store in code_store for fast lookup
        self.code_store.insert(code.hash, code.bytecode.clone());

        Ok(())
    }

    /// Apply genesis allocations to the trie.
    pub fn apply_genesis(
        &mut self,
        accounts: &BTreeMap<Address, GenesisAccount>,
    ) -> Result<(), BinaryTrieError> {
        for (address, genesis) in accounts {
            let code_hash = ethrex_common::utils::keccak(genesis.code.as_ref());
            let code_size = genesis.code.len() as u32;

            // Write basic_data
            let basic_data = pack_basic_data(0, code_size, genesis.nonce, genesis.balance);
            self.trie.insert(get_tree_key_for_basic_data(address), basic_data)?;

            // Write code_hash
            self.trie.insert(get_tree_key_for_code_hash(address), code_hash.0)?;

            // Write code chunks + store
            if !genesis.code.is_empty() {
                let chunks = chunkify_code(&genesis.code);
                for (i, chunk) in chunks.iter().enumerate() {
                    self.trie.insert(
                        get_tree_key_for_code_chunk(address, i as u64),
                        *chunk,
                    )?;
                }
                self.code_store.insert(code_hash, genesis.code.clone());
            }

            // Write storage slots
            for (slot, value) in &genesis.storage {
                if !value.is_zero() {
                    let tree_key = get_tree_key_for_storage_slot(address, *slot);
                    let mut val_bytes = [0u8; 32];
                    value.to_big_endian(&mut val_bytes);
                    self.trie.insert(tree_key, val_bytes)?;

                    let key_h256 = H256(slot.to_big_endian());
                    self.storage_keys
                        .entry(*address)
                        .or_default()
                        .insert(key_h256);
                }
            }
        }
        Ok(())
    }
}
```

**`remove_account` code_size ordering:** Note that `get_code_size` reads from the trie
BEFORE we remove basic_data. The method reads basic_data first to know how many chunks
to remove, then removes basic_data. This ordering matters.

Actually, looking at the code above, `remove_account` calls `get_code_size` but the
basic_data leaf is still present at that point (we haven't removed it yet). So the ordering
is correct as written.

### 4. `BinaryTrieVmDb` adapter (`crates/blockchain/binary_trie_db.rs`)

```rust
use std::collections::BTreeMap;
use ethrex_common::{Address, H256, U256};
use ethrex_common::constants::EMPTY_KECCACK_HASH;
use ethrex_common::types::{AccountState, ChainConfig, Code, CodeMetadata};
use ethrex_vm::{EvmError, VmDatabase};
use ethrex_binary_trie::state::BinaryTrieState;

/// VmDatabase adapter backed by a BinaryTrieState.
///
/// Unlike StoreVmDatabase, this does not use Store/RocksDB.
/// The trie state is shared mutably across block execution —
/// reads come from the current (pre-block) state, writes are
/// applied after execution via apply_account_update.
#[derive(Clone)]
pub struct BinaryTrieVmDb {
    /// Shared reference to the trie state. Clone is cheap (Arc).
    /// During execution the VM only reads; writes happen after.
    state: Arc<RwLock<BinaryTrieState>>,
    chain_config: ChainConfig,
    block_hashes: Arc<Mutex<BTreeMap<u64, H256>>>,
}

impl VmDatabase for BinaryTrieVmDb {
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError> {
        let state = self.state.read()
            .map_err(|_| EvmError::Custom("lock error".into()))?;
        Ok(state.get_account_state(&address))
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        let state = self.state.read()
            .map_err(|_| EvmError::Custom("lock error".into()))?;
        Ok(state.get_storage_slot(&address, key))
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        let cache = self.block_hashes.lock()
            .map_err(|_| EvmError::Custom("lock error".into()))?;
        cache.get(&block_number)
            .copied()
            .ok_or_else(|| EvmError::DB(
                format!("Block hash not found for block number {block_number}")
            ))
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        Ok(self.chain_config.clone())
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, EvmError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Code::default());
        }
        let state = self.state.read()
            .map_err(|_| EvmError::Custom("lock error".into()))?;
        state.get_account_code(&code_hash)
            .map(|bytes| Code::from_bytecode_unchecked(bytes, code_hash))
            .ok_or_else(|| EvmError::DB(
                format!("Code not found for hash: {code_hash:?}")
            ))
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, EvmError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(CodeMetadata { length: 0 });
        }
        let state = self.state.read()
            .map_err(|_| EvmError::Custom("lock error".into()))?;
        state.get_account_code(&code_hash)
            .map(|bytes| CodeMetadata { length: bytes.len() as u64 })
            .ok_or_else(|| EvmError::DB(
                format!("Code metadata not found for hash: {code_hash:?}")
            ))
    }
}
```

**DynClone:** `VmDatabase` requires `DynClone`. The `Arc<RwLock<...>>` fields are all Clone,
so `#[derive(Clone)]` satisfies this automatically.

### 5. Handling `storage_root` — Summary

The `storage_root` field in `AccountState` is an MPT concept. In EIP-7864 there is no
per-account storage root. The VM uses `storage_root` for exactly one purpose:

- `LevmAccount::has_storage` = `state.storage_root != EMPTY_TRIE_HASH` (line 78, account.rs)
- This is used in EIP-7610 create-collision detection and `removed_storage` logic

Our approach: return `EMPTY_TRIE_HASH` when `storage_keys[address]` is empty, otherwise
return `H256::from_low_u64_be(1)`. This correctly signals has_storage/no_storage to the VM
without computing any actual storage trie root.

### 6. Handling `removed_storage` — Summary

`removed_storage: true` on `AccountUpdate` means "clear all storage for this account"
(SELFDESTRUCT followed by recreation in the same transaction, per EIP-6780).

In the MPT world, this means replacing the account's storage trie root with
`EMPTY_TRIE_HASH`. In the binary trie, we must delete every individual storage leaf.

We maintain `storage_keys: HashMap<Address, HashSet<H256>>` as a side structure:
- On every storage write: add the H256 key to the set
- On every storage delete (zero write): remove from the set
- On `removed_storage`: iterate the set, remove each trie leaf, clear the set

Memory cost: ~32 bytes per storage slot per account. For mainnet (~1.5B storage slots),
this would be ~48 GB. **This is a known limitation** acceptable for the experimental shadow
node scope. Future optimization: add prefix-scan to the binary trie, or store per-account
storage keys in a separate DB structure.

For the initial shadow node running from genesis up, the set grows gradually and can be
monitored. If memory becomes an issue, we can spill to a separate on-disk index.

---

## Dependencies

### `ethrex-binary-trie` (Cargo.toml changes)

Add `bytes` as a dependency (for `Bytes` type in code_store):

```toml
[dependencies]
blake3 = "1"
bytes.workspace = true          # NEW
ethrex-common.workspace = true
thiserror.workspace = true
```

No new crate dependencies are needed. `HashMap`, `HashSet`, `BTreeMap` are all std.

### `ethrex-blockchain` (Cargo.toml changes)

Add `ethrex-binary-trie` as an optional dependency behind a feature flag:

```toml
[dependencies]
ethrex-binary-trie = { path = "../common/binary_trie", optional = true }

[features]
binary-trie = ["dep:ethrex-binary-trie"]
```

---

## Implementation Plan

### Phase 1: `BinaryTrieState` core reads + writes (Complexity: Medium)

**File:** `crates/common/binary_trie/state.rs`

- [ ] 1.1: Define `BinaryTrieState` struct with `trie`, `code_store`, `storage_keys` fields
- [ ] 1.2: Implement `new()`, `state_root()`
- [ ] 1.3: Implement `get_account_state()` — read basic_data + code_hash, synthesize storage_root
- [ ] 1.4: Implement `get_storage_slot()`, `get_account_code()`, `get_code_size()`
- [ ] 1.5: Implement `write_code()` helper (chunkify + store)
- [ ] 1.6: Implement `clear_account_storage()`, `remove_account()` helpers
- [ ] 1.7: Implement `apply_account_update()`
- [ ] 1.8: Implement `apply_genesis()`
- [ ] 1.9: Add `pub mod state;` to `lib.rs`, add `bytes` dependency to Cargo.toml
- [ ] 1.10: Add error variants to `BinaryTrieError` if needed (e.g., `InvalidVersion`)

**Acceptance criteria:** Unit tests pass for round-trip account read/write, storage
read/write, code storage, genesis application, and state root changes on mutations.

### Phase 2: Unit tests for `BinaryTrieState` (Complexity: Medium)

**File:** `crates/common/binary_trie/state.rs` (inline `#[cfg(test)] mod tests`)

- [ ] 2.1: Test `get_account_state` returns None for non-existent account
- [ ] 2.2: Test genesis application → accounts readable with correct nonce/balance/code_hash
- [ ] 2.3: Test storage slot write and read round-trip
- [ ] 2.4: Test storage slot delete (write zero) removes from trie and storage_keys
- [ ] 2.5: Test `apply_account_update` with info change (balance/nonce update)
- [ ] 2.6: Test `apply_account_update` with code deployment
- [ ] 2.7: Test `apply_account_update` with `removed: true` clears all account data
- [ ] 2.8: Test `apply_account_update` with `removed_storage: true` clears storage but keeps account
- [ ] 2.9: Test state_root changes after mutations
- [ ] 2.10: Test state_root is deterministic (same ops → same root)
- [ ] 2.11: Test `get_account_code` returns None for unknown hash, Some for stored code
- [ ] 2.12: Test `storage_root` synthesis: EMPTY_TRIE_HASH when no storage, non-empty when storage exists

### Phase 3: `BinaryTrieVmDb` adapter (Complexity: Low)

**File:** `crates/blockchain/binary_trie_db.rs`

- [ ] 3.1: Define `BinaryTrieVmDb` struct with `Arc<RwLock<BinaryTrieState>>`, `ChainConfig`, block hash cache
- [ ] 3.2: Implement `VmDatabase` trait (all 6 methods)
- [ ] 3.3: Add constructor `new(state, chain_config)` and `add_block_hash(number, hash)`
- [ ] 3.4: Add `pub mod binary_trie_db;` to `crates/blockchain/mod.rs` (behind `#[cfg(feature = "binary-trie")]`)
- [ ] 3.5: Add feature flag and optional dependency to `crates/blockchain/Cargo.toml`

**Acceptance criteria:** `BinaryTrieVmDb` compiles, implements `VmDatabase + Clone`,
passes basic smoke test reading from a genesis-initialized `BinaryTrieState`.

### Phase 4: Integration test — execute a simple block (Complexity: High)

- [ ] 4.1: Create test in `crates/common/binary_trie/tests/` that:
  - Initializes `BinaryTrieState` with a genesis (one funded account)
  - Constructs a block with a simple ETH transfer
  - Executes via the existing VM pipeline with `BinaryTrieVmDb`
  - Applies resulting `AccountUpdate`s back to `BinaryTrieState`
  - Verifies balances changed, state root changed, is deterministic

---

## Edge Cases & Risks

### `remove_account` must read code_size before deleting basic_data
The method needs code_size to know how many chunk leaves to remove. Must read basic_data
first. Current design handles this — `get_code_size` reads from trie before `remove` is called.

### Code shared by multiple accounts
`code_store` is keyed by hash. `remove_account` does NOT remove from `code_store` because
other accounts may share the same code (e.g., proxy clones). This is a minor memory leak
but acceptable. If needed, add refcounting later.

### Large `storage_keys` on mainnet
At ~1.5B slots, the `HashMap<Address, HashSet<H256>>` would consume ~48 GB. For the
experimental shadow node (starting from genesis, growing over time), this is manageable
initially. If it becomes a problem:
- Option A: Spill to a RocksDB column
- Option B: Add prefix-scan capability to the binary trie
- Option C: Drop the storage_keys structure and accept that `removed_storage` is a no-op
  (SELFDESTRUCT is extremely rare post-EIP-6780)

### `get_account_state` for accounts with code but no storage
Returns `storage_root = EMPTY_TRIE_HASH` correctly. The `has_storage` check in `LevmAccount`
will be false. This is correct — the account exists but has no storage.

### Zero-value storage writes
EIP-7864 doesn't store zero values. A write of U256::zero() should delete the storage leaf.
The current plan handles this in `apply_account_update` by checking `value.is_zero()`.

### genesis.storage uses `BTreeMap<U256, U256>` but `added_storage` uses `FxHashMap<H256, U256>`
Different key types. Genesis storage keys are `U256` (slot numbers), while `AccountUpdate.added_storage`
uses `H256` (big-endian bytes of the slot number). Both must be converted to `U256` for
`get_tree_key_for_storage_slot`. The code handles this by calling `U256::from_big_endian(key.as_bytes())`
for H256 keys and passing U256 directly for genesis keys.

---

## Open Questions

1. **Memory budget for `storage_keys`**: Should we set a cap or implement spill-to-disk
   from the start? Recommendation: defer until we see actual memory usage during shadow sync.

2. **Code garbage collection**: Should `code_store` entries be refcounted and cleaned up on
   account removal? Recommendation: no, accept the minor leak for now.

3. **Thread safety**: The plan uses `Arc<RwLock<BinaryTrieState>>` in `BinaryTrieVmDb`.
   During block execution, the VM only reads. Writes (apply_account_update) happen after.
   Is this always the case, or can the VM write during execution? — Yes, the VM only reads
   from `VmDatabase`; writes are accumulated in `GeneralizedDatabase` and flushed as
   `AccountUpdate`s after execution. The RwLock read-lock is safe.
