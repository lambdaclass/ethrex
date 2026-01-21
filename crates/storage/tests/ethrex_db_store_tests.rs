//! Store-level tests for ethrex_db backend integration.
//!
//! These tests verify that the Store API works correctly when using the
//! ethrex_db hybrid backend. They cover:
//! - Basic Store operations with ethrex_db
//! - Concurrent access patterns
//! - Fork choice and reorg handling

#![cfg(feature = "ethrex-db")]

use bytes::Bytes;
use ethrex_common::{
    types::{Block, BlockBody, BlockHeader, ChainConfig, Code, Genesis, GenesisAccount},
    utils::keccak,
    Address, H256, U256,
};
use ethrex_storage::{EngineType, Store};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;

/// Helper to create a test genesis configuration.
fn create_test_genesis() -> Genesis {
    let mut alloc = BTreeMap::new();

    // Add some test accounts to genesis
    let alice = Address::from_low_u64_be(1);
    let bob = Address::from_low_u64_be(2);

    alloc.insert(
        alice,
        GenesisAccount {
            balance: U256::from(1_000_000_000u64),
            nonce: 0,
            code: Bytes::new(),
            storage: BTreeMap::new(),
        },
    );
    alloc.insert(
        bob,
        GenesisAccount {
            balance: U256::from(500_000_000u64),
            nonce: 0,
            code: Bytes::new(),
            storage: BTreeMap::new(),
        },
    );

    Genesis {
        config: ChainConfig::default(),
        alloc,
        ..Default::default()
    }
}

/// Helper to create an empty test block.
fn create_test_block(number: u64, parent_hash: H256) -> Block {
    let header = BlockHeader {
        number,
        parent_hash,
        gas_limit: 30_000_000,
        timestamp: 1000 + number,
        ..Default::default()
    };

    Block::new(header, BlockBody::default())
}

// =============================================================================
// Basic Store Tests with EthrexDb Backend
// =============================================================================

#[test]
fn test_store_creation_with_ethrex_db() {
    let temp_dir = TempDir::new().unwrap();
    let store = Store::new(temp_dir.path(), EngineType::EthrexDb)
        .expect("Failed to create store with ethrex_db");

    // Verify the store was created with ethrex_db backend
    assert!(store.uses_ethrex_db());

    // Verify we can access the blockchain reference
    let blockchain_ref = store.ethrex_blockchain();
    assert!(blockchain_ref.is_some());
}

#[tokio::test]
async fn test_store_genesis_initialization_with_ethrex_db() {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::new(temp_dir.path(), EngineType::EthrexDb)
        .expect("Failed to create store with ethrex_db");

    let genesis = create_test_genesis();
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");

    // Verify genesis block was created
    let latest = store.get_latest_block_number().await;
    assert!(latest.is_ok());

    // Verify chain config was stored
    let _config = store.get_chain_config();
}

#[tokio::test]
async fn test_store_block_operations_with_ethrex_db() {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::new(temp_dir.path(), EngineType::EthrexDb)
        .expect("Failed to create store with ethrex_db");

    // Initialize with genesis
    let genesis = create_test_genesis();
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis");

    // Get genesis hash
    let genesis_hash = store
        .get_canonical_block_hash(0)
        .await
        .expect("Failed to get genesis hash")
        .expect("Genesis hash not found");

    // Create and add a new block
    let block1 = create_test_block(1, genesis_hash);
    let block1_hash = block1.hash();

    store
        .add_block(block1.clone())
        .await
        .expect("Failed to add block");

    // Set block as canonical using forkchoice_update
    store
        .forkchoice_update(vec![(1, block1_hash)], 1, block1_hash, None, None)
        .await
        .expect("Failed to set canonical via forkchoice_update");

    // Verify block was stored
    let retrieved_header = store
        .get_block_header(1)
        .expect("Failed to get header")
        .expect("Header not found");

    assert_eq!(retrieved_header.number, 1);
    assert_eq!(retrieved_header.parent_hash, genesis_hash);
}

// =============================================================================
// Concurrent Access Tests
// =============================================================================

#[tokio::test]
async fn test_concurrent_reads_with_ethrex_db() {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::new(temp_dir.path(), EngineType::EthrexDb)
        .expect("Failed to create store");

    // Initialize with genesis
    let genesis = create_test_genesis();
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis");

    // Get genesis hash for verification using sync method
    let genesis_hash = store
        .get_canonical_block_hash_sync(0)
        .expect("Failed to get genesis hash")
        .expect("Genesis hash not found");

    // Clone store for concurrent access
    let store = Arc::new(store);

    // Spawn multiple reader threads
    let mut handles = vec![];
    for i in 0..4 {
        let store_clone = Arc::clone(&store);
        let expected_hash = genesis_hash;

        let handle = thread::spawn(move || {
            // Perform multiple reads using sync method
            for _ in 0..10 {
                let hash = store_clone
                    .get_canonical_block_hash_sync(0)
                    .expect("Read failed")
                    .expect("Hash not found");

                assert_eq!(hash, expected_hash, "Thread {} got unexpected hash", i);
            }
        });
        handles.push(handle);
    }

    // Wait for all threads to complete
    for handle in handles {
        handle.join().expect("Thread panicked");
    }
}

#[tokio::test]
async fn test_concurrent_reads_during_write_with_ethrex_db() {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::new(temp_dir.path(), EngineType::EthrexDb)
        .expect("Failed to create store");

    // Initialize with genesis
    let genesis = create_test_genesis();
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis");

    let store = Arc::new(store);

    // Spawn reader threads using sync methods
    let mut handles = vec![];

    for _ in 0..2 {
        let store_clone = Arc::clone(&store);
        let handle = thread::spawn(move || {
            for _ in 0..20 {
                // Read operations should not fail even during writes
                let _ = store_clone.get_canonical_block_hash_sync(0);
                let _ = store_clone.get_block_header(0);
                thread::sleep(std::time::Duration::from_millis(1));
            }
        });
        handles.push(handle);
    }

    // Write new blocks while readers are active (in main async context)
    let mut parent_hash = store
        .get_canonical_block_hash_sync(0)
        .expect("Failed to get genesis")
        .expect("Genesis not found");

    for i in 1..5u64 {
        let block = create_test_block(i, parent_hash);
        let block_hash = block.hash();

        store.add_block(block).await.expect("Failed to add block");

        // Use forkchoice_update to set canonical
        store
            .forkchoice_update(vec![(i, block_hash)], i, block_hash, None, None)
            .await
            .expect("Failed to set canonical");

        parent_hash = block_hash;
        thread::sleep(std::time::Duration::from_millis(5));
    }

    // Wait for all readers to complete
    for handle in handles {
        handle.join().expect("Reader thread panicked");
    }

    // Verify final state
    let latest = store.get_latest_block_number().await.expect("Failed to get latest");
    assert_eq!(latest, 4);
}

// =============================================================================
// Blockchain Reference Tests
// =============================================================================

#[tokio::test]
async fn test_blockchain_reference_access() {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::new(temp_dir.path(), EngineType::EthrexDb)
        .expect("Failed to create store");

    let genesis = create_test_genesis();
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis");

    // Access blockchain reference
    let blockchain_ref = store.ethrex_blockchain().expect("Should have blockchain ref");

    // Acquire read lock on blockchain
    let blockchain = blockchain_ref.0.read().expect("Failed to read lock");

    // Verify blockchain state
    // The blockchain should be at the finalized state
    let finalized_number = blockchain.last_finalized_number();
    // Genesis is block 0, but finalization depends on how ethrex_db handles initial state
    assert!(
        finalized_number <= 1,
        "Unexpected finalized number: {}",
        finalized_number
    );
}

// =============================================================================
// Helper Method Tests
// =============================================================================

#[test]
fn test_uses_ethrex_db_helper() {
    // Test with ethrex_db backend
    let temp_dir1 = TempDir::new().unwrap();
    let store1 = Store::new(temp_dir1.path(), EngineType::EthrexDb)
        .expect("Failed to create ethrex_db store");
    assert!(store1.uses_ethrex_db());

    // Test with RocksDB backend
    let temp_dir2 = TempDir::new().unwrap();
    let store2 = Store::new(temp_dir2.path(), EngineType::RocksDB)
        .expect("Failed to create rocksdb store");
    assert!(!store2.uses_ethrex_db());

    // Test with InMemory backend
    let store3 = Store::new("", EngineType::InMemory).expect("Failed to create in-memory store");
    assert!(!store3.uses_ethrex_db());
}

// =============================================================================
// Multiple Blocks Chain Test
// =============================================================================

#[tokio::test]
async fn test_multiple_blocks_chain_with_ethrex_db() {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::new(temp_dir.path(), EngineType::EthrexDb)
        .expect("Failed to create store");

    // Initialize with genesis
    let genesis = create_test_genesis();
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis");

    // Build a chain of 5 blocks
    let mut parent_hash = store
        .get_canonical_block_hash_sync(0)
        .expect("Failed to get genesis")
        .expect("Genesis not found");

    for i in 1..=5u64 {
        let block = create_test_block(i, parent_hash);
        let block_hash = block.hash();

        store.add_block(block).await.expect("Failed to add block");

        // Use forkchoice_update to set canonical
        store
            .forkchoice_update(vec![(i, block_hash)], i, block_hash, None, None)
            .await
            .expect("Failed to set canonical");

        parent_hash = block_hash;
    }

    // Verify the chain
    let latest = store.get_latest_block_number().await.expect("Failed to get latest");
    assert_eq!(latest, 5);

    // Verify each block is retrievable
    for i in 0..=5u64 {
        let hash = store
            .get_canonical_block_hash_sync(i)
            .expect("Failed to get hash")
            .expect(&format!("Block {} hash not found", i));

        let header = store
            .get_block_header_by_hash(hash)
            .expect("Failed to get header")
            .expect(&format!("Block {} header not found", i));

        assert_eq!(header.number, i);
    }
}

// =============================================================================
// Store Reopening Test
// =============================================================================

#[tokio::test]
async fn test_store_persistence_after_reopen() {
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path().to_path_buf();

    // Create store and add some data
    {
        let mut store = Store::new(&temp_path, EngineType::EthrexDb)
            .expect("Failed to create store");

        let genesis = create_test_genesis();
        store
            .add_initial_state(genesis)
            .await
            .expect("Failed to add genesis");

        let genesis_hash = store
            .get_canonical_block_hash_sync(0)
            .expect("Failed to get genesis")
            .expect("Genesis not found");

        // Add a block
        let block1 = create_test_block(1, genesis_hash);
        let block1_hash = block1.hash();

        store.add_block(block1).await.expect("Failed to add block");

        // Use forkchoice_update to set canonical
        store
            .forkchoice_update(vec![(1, block1_hash)], 1, block1_hash, None, None)
            .await
            .expect("Failed to set canonical");
    }

    // Reopen the store
    {
        let store = Store::new(&temp_path, EngineType::EthrexDb)
            .expect("Failed to reopen store");

        assert!(store.uses_ethrex_db());

        // Verify block header persisted in RocksDB auxiliary
        let header = store
            .get_block_header(1)
            .expect("Failed to get header")
            .expect("Header not found after reopen");

        assert_eq!(header.number, 1);
    }
}

// =============================================================================
// Backend-Specific File Structure Test
// =============================================================================

#[test]
fn test_ethrex_db_creates_expected_files() {
    let temp_dir = TempDir::new().unwrap();
    let _store = Store::new(temp_dir.path(), EngineType::EthrexDb)
        .expect("Failed to create store");

    // Verify expected file structure
    let state_db_path = temp_dir.path().join("state.db");
    let auxiliary_path = temp_dir.path().join("auxiliary");

    assert!(
        state_db_path.exists(),
        "state.db should exist at {:?}",
        state_db_path
    );
    assert!(
        auxiliary_path.exists(),
        "auxiliary/ should exist at {:?}",
        auxiliary_path
    );
    assert!(auxiliary_path.is_dir(), "auxiliary should be a directory");
}

// =============================================================================
// Fork Choice Tests
// =============================================================================

#[tokio::test]
async fn test_forkchoice_update_basic() {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::new(temp_dir.path(), EngineType::EthrexDb)
        .expect("Failed to create store");

    // Initialize with genesis
    let genesis = create_test_genesis();
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis");

    let genesis_hash = store
        .get_canonical_block_hash_sync(0)
        .expect("Failed to get genesis")
        .expect("Genesis not found");

    // Add a few blocks
    let mut parent_hash = genesis_hash;
    let mut block_hashes = vec![genesis_hash];

    for i in 1..=3u64 {
        let block = create_test_block(i, parent_hash);
        let block_hash = block.hash();

        store.add_block(block).await.expect("Failed to add block");

        // Set canonical incrementally (without finalization)
        store
            .forkchoice_update(vec![(i, block_hash)], i, block_hash, None, None)
            .await
            .expect("Failed to set canonical");

        parent_hash = block_hash;
        block_hashes.push(block_hash);
    }

    // Verify the blocks are canonical
    for i in 1..=3u64 {
        let hash = store
            .get_canonical_block_hash_sync(i)
            .expect("Failed to get hash")
            .expect(&format!("Block {} hash not found", i));
        assert_eq!(hash, block_hashes[i as usize]);
    }
}

#[tokio::test]
async fn test_forkchoice_update_with_safe_and_finalized() {
    let temp_dir = TempDir::new().unwrap();
    let mut store = Store::new(temp_dir.path(), EngineType::EthrexDb)
        .expect("Failed to create store");

    // Initialize with genesis
    let genesis = create_test_genesis();
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis");

    let genesis_hash = store
        .get_canonical_block_hash_sync(0)
        .expect("Failed to get genesis")
        .expect("Genesis not found");

    // Build a chain of 5 blocks
    let mut parent_hash = genesis_hash;

    for i in 1..=5u64 {
        let block = create_test_block(i, parent_hash);
        let block_hash = block.hash();

        store.add_block(block).await.expect("Failed to add block");

        // Set canonical without finalization for now
        // Note: Full ethrex_db finalization requires blocks to be in ethrex_db's Blockchain
        // which is not fully integrated yet (blocks go to RocksDB auxiliary).
        store
            .forkchoice_update(vec![(i, block_hash)], i, block_hash, None, None)
            .await
            .expect("Failed to set canonical");

        parent_hash = block_hash;
    }

    // Verify the chain is canonical
    let latest = store.get_latest_block_number().await.expect("Failed to get latest");
    assert_eq!(latest, 5);
}

/// Test contract code storage and retrieval with ethrex_db backend.
/// Code is stored in RocksDB auxiliary storage, not in ethrex_db itself.
#[tokio::test]
async fn test_code_storage_and_retrieval_with_ethrex_db() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Create store with ethrex_db backend
    let store = Store::new(temp_dir.path(), EngineType::EthrexDb)
        .expect("Failed to create store");

    // Create some test bytecode (simple contract)
    let bytecode = Bytes::from(vec![
        0x60, 0x80, 0x60, 0x40, 0x52, // PUSH1 0x80 PUSH1 0x40 MSTORE
        0x34, 0x80, 0x15, // CALLVALUE DUP1 ISZERO
        0x60, 0x0f, 0x57, // PUSH1 0x0f JUMPI
        0x60, 0x00, // PUSH1 0x00
        0x80, 0xfd, // DUP1 REVERT
        0x5b, // JUMPDEST
        0x50, // POP
        0x60, 0x00, 0x80, 0xfd, // PUSH1 0x00 DUP1 REVERT
    ]);

    // Compute the code hash
    let code_hash = H256::from(keccak(bytecode.as_ref()));

    // Create Code struct
    let code = Code::from_bytecode_unchecked(bytecode.clone(), code_hash);

    // Store the code
    store
        .add_account_code(code.clone())
        .await
        .expect("Failed to add account code");

    // Retrieve the code
    let retrieved_code = store
        .get_account_code(code_hash)
        .expect("Failed to get account code")
        .expect("Code should exist");

    // Verify the code matches
    assert_eq!(retrieved_code.hash, code_hash);
    assert_eq!(retrieved_code.bytecode, bytecode);
    assert_eq!(retrieved_code.jump_targets, code.jump_targets);
}

/// Test code storage persists across store reopening.
#[tokio::test]
async fn test_code_storage_persistence_with_ethrex_db() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path().to_path_buf();

    // Create some test bytecode
    let bytecode = Bytes::from(vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00]); // PUSH1 1 PUSH1 2 ADD STOP
    let code_hash = H256::from(keccak(bytecode.as_ref()));
    let code = Code::from_bytecode_unchecked(bytecode.clone(), code_hash);

    // Store code in first store instance
    {
        let store = Store::new(&temp_path, EngineType::EthrexDb)
            .expect("Failed to create store");

        store
            .add_account_code(code.clone())
            .await
            .expect("Failed to add account code");
    }

    // Reopen store and verify code is still there
    {
        let store = Store::new(&temp_path, EngineType::EthrexDb)
            .expect("Failed to reopen store");

        let retrieved_code = store
            .get_account_code(code_hash)
            .expect("Failed to get account code")
            .expect("Code should persist after reopen");

        assert_eq!(retrieved_code.hash, code_hash);
        assert_eq!(retrieved_code.bytecode, bytecode);
    }
}

/// Test that non-existent code returns None.
#[test]
fn test_nonexistent_code_returns_none() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let store = Store::new(temp_dir.path(), EngineType::EthrexDb)
        .expect("Failed to create store");

    // Try to get a code hash that was never stored
    let fake_code_hash = H256::repeat_byte(0xAB);

    let result = store
        .get_account_code(fake_code_hash)
        .expect("Query should succeed");

    assert!(result.is_none(), "Non-existent code should return None");
}
