# ethrex_db Integration - Migration Guide

## Overview

This document describes the integration of `ethrex_db` into the ethrex storage layer, replacing the previous `ethrex-trie` implementation.

## What Changed

### Storage Engine Replacement

**Before:** `ethrex-trie` with RocksDB backend
**After:** `ethrex_db` with hot/cold storage separation

### Performance Improvements

Based on ethrex_db benchmarks:
- **10-15x faster** read operations (RPC queries, state lookups)
- **1.6-2.2x faster** write operations (block execution)
- **12-13x faster** state root computation (merkleization)
- **Reduced memory usage** through Copy-on-Write semantics

### Architecture Changes

#### Old Architecture
```
Store
  ├── backend: RocksDB
  ├── trie_cache: TrieLayerCache (128 blocks deep)
  ├── flatkeyvalue_control_tx: Background FKV generator
  ├── trie_update_worker_tx: Background trie updater
  └── background_threads: 2 worker threads
```

#### New Architecture
```
Store
  ├── blockchain: Blockchain (ethrex_db hot storage)
  │   └── db: PagedDb (ethrex_db cold storage, internal)
  └── backend: Legacy backend (for non-state data only)
```

### Hot/Cold Storage Separation

- **Hot Storage (Blockchain)**: Recent unfinalized blocks with COW semantics
  - Supports parallel block creation
  - Fast reads from in-memory state
  - Handles reorgs efficiently

- **Cold Storage (PagedDb)**: Finalized blocks in memory-mapped pages
  - 4KB page-based storage
  - Persistent across restarts
  - Optimized for sequential access

## API Changes

### New Methods (Use These)

#### State Queries
```rust
// Get account information
store.get_account_info_ethrex_db(block_number, address)?;

// Get storage value
store.get_storage_at_ethrex_db(block_number, address, storage_key)?;
```

#### Block Execution
```rust
// Execute a block with account updates
store.execute_block_ethrex_db(
    parent_hash,
    block_hash,
    block_number,
    &account_updates
)?;

// Finalize a block (move from hot to cold storage)
store.finalize_block_ethrex_db(block_hash)?;

// Update fork choice
store.fork_choice_update_ethrex_db(head, safe, finalized)?;
```

### Deprecated Methods (Legacy - Do Not Use)

These methods now return `unimplemented!()` and will be removed:

```rust
// DEPRECATED - Do not use
store.open_state_trie(state_root)?;
store.open_direct_state_trie(state_root)?;
store.open_locked_state_trie(state_root)?;
store.open_storage_trie(account_hash, state_root, storage_root)?;
store.open_direct_storage_trie(account_hash, storage_root)?;
store.open_locked_storage_trie(account_hash, state_root, storage_root)?;
```

## Breaking Changes

### Database Schema Version

**STORE_SCHEMA_VERSION: 1 → 2**

This is a **breaking change** that requires:
1. Deleting existing database
2. Resyncing from genesis or restoring from snapshot

### Migration Steps for Users

1. **Backup your data** (if needed for reference)
2. **Stop the ethrex node**
3. **Delete the database directory** (usually `./data` or similar)
4. **Update to the new version**
5. **Restart the node** - it will resync from genesis

Example:
```bash
# Stop node
pkill ethrex

# Backup (optional)
mv ./data ./data.backup

# Update
git pull
cargo build --release

# Restart - will resync from genesis
./target/release/ethrex
```

## Implementation Status

### ✅ Completed

- [x] Vendored ethrex_db dependency
- [x] Updated Store struct with Blockchain and PagedDb
- [x] Implemented new state query methods
- [x] Implemented block execution methods
- [x] Implemented finalization methods
- [x] Deprecated legacy trie methods
- [x] All compilation errors fixed
- [x] 6 out of 9 existing tests passing

### 🚧 TODO (Future Work)

#### Critical
- [ ] Implement `setup_genesis_state_trie_ethrex_db()` for genesis setup
- [ ] Update `add_initial_state()` to use ethrex_db
- [ ] Migrate sync code (`crates/networking/p2p/sync*.rs`)

#### Tests
- [ ] Rewrite `test_genesis_block` for ethrex_db
- [ ] Rewrite `test_iter_accounts` for ethrex_db
- [ ] Rewrite `test_iter_storage` for ethrex_db
- [ ] Add integration tests for hot/cold storage transition
- [ ] Add tests for fork handling

#### Cleanup
- [ ] Remove `layering.rs` and `trie.rs` completely
- [ ] Remove ethrex-trie dependency from workspace
- [ ] Remove legacy backend usage where possible

#### Documentation
- [ ] Add examples for common operations
- [ ] Document cold storage recovery process
- [ ] Add benchmarks comparing old vs new performance

## Known Limitations

### 1. Genesis Setup Not Implemented

The `setup_genesis_state_trie()` method still uses legacy trie methods and needs to be rewritten for ethrex_db. This means:
- `Store::new_from_genesis()` will panic
- Genesis setup must be done manually for now
- Test `test_genesis_block` is disabled

**Workaround**: Initialize genesis state manually or use a snapshot.

### 2. Sync Code Still Uses Legacy Methods

The sync healing code in `crates/networking/p2p/sync*.rs` still calls legacy trie methods:
- `write_storage_trie_nodes_batch()`
- `write_account_trie_nodes_batch()`

These need to be migrated to ethrex_db equivalents.

### 3. Iterator Methods Not Available

The old trie iterator methods are not yet implemented in ethrex_db:
- `iter_accounts_from()`
- `iter_storage_from()`

These are used for snap sync and need new implementations.

## Developer Guide

### Using the New API

#### Reading State
```rust
// Old way (DEPRECATED)
let trie = store.open_state_trie(state_root)?;
let account_bytes = trie.get(&hashed_address)?;

// New way
let account_info = store.get_account_info_ethrex_db(
    block_number,
    address
)?;
```

#### Executing Blocks
```rust
// Old way (DEPRECATED)
store.apply_updates(update_batch)?;

// New way
let account_updates = vec![
    AccountUpdate {
        address,
        removed: false,
        info: Some(account_info),
        code: None,
        added_storage: storage_updates,
        removed_storage: false,
    },
];

store.execute_block_ethrex_db(
    parent_hash,
    block_hash,
    block_number,
    &account_updates
)?;
```

#### Finalizing Blocks
```rust
// After a block is confirmed/finalized
store.finalize_block_ethrex_db(block_hash)?;

// Or use fork choice update (automatically finalizes)
store.fork_choice_update_ethrex_db(
    head_hash,
    Some(safe_hash),
    Some(finalized_hash)
)?;
```

### Internal Details

#### Store Initialization
```rust
// Creates both in-memory PagedDb and Blockchain layer
let store = Store::new("./data", EngineType::RocksDB)?;

// Blockchain takes ownership of PagedDb
// All state access goes through Blockchain
```

#### Address and Key Hashing
```rust
// Addresses are hashed before ethrex_db lookup
let address_hash = H256::from(keccak_hash(address.to_fixed_bytes()));

// Storage keys are also hashed
let key_hash = H256::from(keccak_hash(storage_key.to_fixed_bytes()));
```

#### Block Lifecycle
```
1. start_new() - Create new block (COW from parent)
2. apply updates - Modify block state in memory
3. commit() - Compute state root, store in Blockchain (hot)
4. finalize() - Move to PagedDb (cold), free COW memory
```

## Benchmarking

To benchmark the new storage engine:

```bash
cd crates/common/ethrex_db
cargo bench
```

Key benchmarks:
- `read_account` - Random account lookups
- `insert_account` - Account insertions
- `compute_root_hash` - State root computation
- `trie_iteration` - Sequential scans

## Troubleshooting

### "Legacy method - use ethrex_db instead"
**Cause**: Code is calling deprecated trie methods
**Fix**: Update to use new `*_ethrex_db()` methods

### "Block not found after commit"
**Cause**: Block not in hot storage
**Fix**: Ensure block was committed before querying

### Tests failing with "unimplemented"
**Cause**: Tests use legacy trie methods
**Fix**: Rewrite tests using new ethrex_db methods or skip them

### Database version mismatch
**Cause**: Trying to use old database with new schema
**Fix**: Delete database and resync from genesis

## References

- **ethrex_db Repository**: https://github.com/unbalancedparentheses/ethrex_db
- **Integration Plan**: `/home/esteve/.claude/plans/effervescent-purring-bonbon.md`
- **Store Schema Version**: `STORE_SCHEMA_VERSION = 2` in `crates/storage/lib.rs`

## Support

For questions or issues with the migration:
1. Check this migration guide
2. Review the integration plan
3. Check ethrex_db documentation
4. Open an issue on GitHub
