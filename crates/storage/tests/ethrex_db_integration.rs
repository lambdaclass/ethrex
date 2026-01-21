//! Integration tests for ethrex-db within the ethrex storage crate.
//!
//! These tests verify that ethrex-db works correctly as a dependency and demonstrate
//! the basic API usage patterns that will be needed for full integration.

#![cfg(feature = "ethrex-db")]

use ethrex_db::chain::{Account, Blockchain, ReadOnlyWorldState, WorldState};
use ethrex_db::merkle::{MerkleTrie, EMPTY_ROOT};
use ethrex_db::store::{PagedDb, PagedStateTrie};
use primitive_types::{H256, U256};

/// Test basic PagedDb creation and operations.
#[test]
fn test_paged_db_in_memory() {
    // Create an in-memory database with 1000 pages (4MB)
    let db = PagedDb::in_memory(1000).expect("Failed to create in-memory PagedDb");

    // Verify initial state
    assert_eq!(db.block_number(), 0);
    assert_eq!(db.block_hash(), [0u8; 32]);
}

/// Test basic Blockchain creation and block operations.
#[test]
fn test_blockchain_creation() {
    let db = PagedDb::in_memory(1000).expect("Failed to create database");
    let blockchain = Blockchain::new(db);

    // Initial state should be at block 0 with zero hash
    assert_eq!(blockchain.last_finalized_number(), 0);
    assert_eq!(blockchain.last_finalized_hash(), H256::zero());
    assert_eq!(blockchain.committed_count(), 0);
}

/// Test creating and committing blocks.
#[test]
fn test_block_creation_and_commit() {
    let db = PagedDb::in_memory(1000).expect("Failed to create database");
    let blockchain = Blockchain::new(db);

    // Create a new block on top of finalized state
    let parent_hash = blockchain.last_finalized_hash();
    let block1_hash = H256::repeat_byte(0x01);

    let block1 = blockchain
        .start_new(parent_hash, block1_hash, 1)
        .expect("Failed to create block");

    assert_eq!(block1.number(), 1);
    assert_eq!(block1.hash(), block1_hash);

    // Commit the block
    blockchain.commit(block1).expect("Failed to commit block");
    assert_eq!(blockchain.committed_count(), 1);
}

/// Test account state management within blocks.
#[test]
fn test_account_state_in_blocks() {
    let db = PagedDb::in_memory(1000).expect("Failed to create database");
    let blockchain = Blockchain::new(db);

    let parent_hash = blockchain.last_finalized_hash();
    let block1_hash = H256::repeat_byte(0x01);

    let mut block1 = blockchain
        .start_new(parent_hash, block1_hash, 1)
        .expect("Failed to create block");

    // Create account addresses (as H256 - hashed addresses)
    let alice = H256::from_low_u64_be(1);
    let bob = H256::from_low_u64_be(2);

    // Set accounts in the block
    block1.set_account(
        alice,
        Account {
            nonce: 0,
            balance: U256::from(1_000_000_000u64),
            storage_root: H256::from(EMPTY_ROOT),
            code_hash: H256::zero(),
        },
    );

    block1.set_account(
        bob,
        Account {
            nonce: 0,
            balance: U256::from(500_000_000u64),
            storage_root: H256::from(EMPTY_ROOT),
            code_hash: H256::zero(),
        },
    );

    // Read back accounts from block (before commit)
    let alice_acc = block1.get_account(&alice).expect("Alice should exist");
    assert_eq!(alice_acc.balance, U256::from(1_000_000_000u64));
    assert_eq!(alice_acc.nonce, 0);

    let bob_acc = block1.get_account(&bob).expect("Bob should exist");
    assert_eq!(bob_acc.balance, U256::from(500_000_000u64));

    // Commit the block
    blockchain.commit(block1).expect("Failed to commit");

    // Read accounts from committed block via blockchain
    let alice_acc = blockchain
        .get_account(&block1_hash, &alice)
        .expect("Alice should exist in committed block");
    assert_eq!(alice_acc.balance, U256::from(1_000_000_000u64));
}

/// Test storage slot operations within blocks.
#[test]
fn test_storage_in_blocks() {
    let db = PagedDb::in_memory(1000).expect("Failed to create database");
    let blockchain = Blockchain::new(db);

    let parent_hash = blockchain.last_finalized_hash();
    let block1_hash = H256::repeat_byte(0x01);

    let mut block1 = blockchain
        .start_new(parent_hash, block1_hash, 1)
        .expect("Failed to create block");

    // Create a contract address
    let contract = H256::from_low_u64_be(0x1000);
    let slot_key = H256::from_low_u64_be(1);
    let slot_value = U256::from(42u64);

    // Set contract account
    block1.set_account(
        contract,
        Account {
            nonce: 1,
            balance: U256::zero(),
            storage_root: H256::from(EMPTY_ROOT),
            code_hash: H256::from_low_u64_be(0xDEADBEEF),
        },
    );

    // Set storage slot
    block1.set_storage(contract, slot_key, slot_value);

    // Read storage back
    let read_value = block1
        .get_storage(&contract, &slot_key)
        .expect("Storage slot should exist");
    assert_eq!(read_value, slot_value);

    // Commit and verify via blockchain
    blockchain.commit(block1).expect("Failed to commit");

    let read_value = blockchain
        .get_storage(&block1_hash, &contract, &slot_key)
        .expect("Storage should exist in committed block");
    assert_eq!(read_value, slot_value);
}

/// Test block finalization and state persistence.
#[test]
fn test_block_finalization() {
    let db = PagedDb::in_memory(1000).expect("Failed to create database");
    let blockchain = Blockchain::new(db);

    let parent_hash = blockchain.last_finalized_hash();
    let block1_hash = H256::repeat_byte(0x01);

    let mut block1 = blockchain
        .start_new(parent_hash, block1_hash, 1)
        .expect("Failed to create block");

    // Add some state
    let alice = H256::from_low_u64_be(1);
    block1.set_account(
        alice,
        Account {
            nonce: 5,
            balance: U256::from(1_000_000u64),
            storage_root: H256::from(EMPTY_ROOT),
            code_hash: H256::zero(),
        },
    );

    blockchain.commit(block1).expect("Failed to commit");
    assert_eq!(blockchain.committed_count(), 1);

    // Finalize the block
    blockchain.finalize(block1_hash).expect("Failed to finalize");

    // Verify finalization
    assert_eq!(blockchain.last_finalized_number(), 1);
    assert_eq!(blockchain.last_finalized_hash(), block1_hash);
    assert_eq!(blockchain.committed_count(), 0); // Block moved to finalized state

    // Read from finalized state
    // Note: get_finalized_account takes a 20-byte address, not H256
    // The finalized account lookup uses the last 20 bytes of H256 as address
    // For this test, we verify the state root changed (state was persisted)
    let state_root = blockchain.state_root();
    assert_ne!(state_root, EMPTY_ROOT);
}

/// Test parallel block creation (fork handling).
#[test]
fn test_parallel_blocks() {
    let db = PagedDb::in_memory(1000).expect("Failed to create database");
    let blockchain = Blockchain::new(db);

    let parent_hash = blockchain.last_finalized_hash();

    // Create two blocks with the same parent (simulating a fork)
    let block1_hash = H256::repeat_byte(0x01);
    let block2_hash = H256::repeat_byte(0x02);

    let block1 = blockchain
        .start_new(parent_hash, block1_hash, 1)
        .expect("Failed to create block 1");

    let block2 = blockchain
        .start_new(parent_hash, block2_hash, 1)
        .expect("Failed to create block 2");

    // Both blocks can be committed
    blockchain.commit(block1).expect("Failed to commit block 1");
    blockchain.commit(block2).expect("Failed to commit block 2");

    assert_eq!(blockchain.committed_count(), 2);

    // Both blocks should be retrievable
    assert!(blockchain.get_block(&block1_hash).is_some());
    assert!(blockchain.get_block(&block2_hash).is_some());
}

/// Test fork choice update.
#[test]
fn test_fork_choice_update() {
    let db = PagedDb::in_memory(1000).expect("Failed to create database");
    let blockchain = Blockchain::new(db);

    let parent_hash = blockchain.last_finalized_hash();
    let block1_hash = H256::repeat_byte(0x01);

    let block1 = blockchain
        .start_new(parent_hash, block1_hash, 1)
        .expect("Failed to create block");

    blockchain.commit(block1).expect("Failed to commit");

    // Fork choice update selecting block1 as head
    blockchain
        .fork_choice_update(block1_hash, None, None)
        .expect("Fork choice update should succeed");

    // Fork choice update with finalization
    blockchain
        .fork_choice_update(block1_hash, None, Some(block1_hash))
        .expect("Fork choice with finalization should succeed");

    assert_eq!(blockchain.last_finalized_hash(), block1_hash);
}

/// Test MerkleTrie for state root computation.
#[test]
fn test_merkle_trie_root_computation() {
    let mut trie = MerkleTrie::new();

    // Empty trie should have empty root
    assert_eq!(trie.root_hash(), EMPTY_ROOT);

    // Insert some values
    trie.insert(b"key1", b"value1".to_vec());
    trie.insert(b"key2", b"value2".to_vec());

    // Root should now be non-empty
    let root1 = trie.root_hash();
    assert_ne!(root1, EMPTY_ROOT);

    // Same insertions should produce same root
    let mut trie2 = MerkleTrie::new();
    trie2.insert(b"key1", b"value1".to_vec());
    trie2.insert(b"key2", b"value2".to_vec());

    assert_eq!(trie2.root_hash(), root1);

    // Different values should produce different root
    let mut trie3 = MerkleTrie::new();
    trie3.insert(b"key1", b"different".to_vec());
    trie3.insert(b"key2", b"value2".to_vec());

    assert_ne!(trie3.root_hash(), root1);
}

/// Test PagedStateTrie for persistent state trie.
#[test]
fn test_paged_state_trie() {
    let mut state_trie = PagedStateTrie::new();

    // Initial state should have empty root
    let initial_root = state_trie.root_hash();
    assert_eq!(initial_root, EMPTY_ROOT);

    // Add an account
    let addr: [u8; 20] = [1; 20];
    let account_data = ethrex_db::store::AccountData {
        nonce: 1,
        balance: [0; 32], // U256 as bytes
        code_hash: [0; 32],
        storage_root: EMPTY_ROOT,
    };

    state_trie.set_account(&addr, account_data);

    // Root should now be different
    let new_root = state_trie.root_hash();
    assert_ne!(new_root, initial_root);

    // Read account back
    let read_account = state_trie.get_account(&addr).expect("Account should exist");
    assert_eq!(read_account.nonce, 1);
}

/// Test multi-block chain with state transitions.
#[test]
fn test_multi_block_chain() {
    let db = PagedDb::in_memory(1000).expect("Failed to create database");
    let blockchain = Blockchain::new(db);

    let alice = H256::from_low_u64_be(1);
    let bob = H256::from_low_u64_be(2);

    // Block 1: Create accounts
    let genesis_hash = blockchain.last_finalized_hash();
    let block1_hash = H256::repeat_byte(0x01);

    let mut block1 = blockchain
        .start_new(genesis_hash, block1_hash, 1)
        .expect("Failed to create block 1");

    block1.set_account(
        alice,
        Account {
            nonce: 0,
            balance: U256::from(1000u64),
            storage_root: H256::from(EMPTY_ROOT),
            code_hash: H256::zero(),
        },
    );
    block1.set_account(
        bob,
        Account {
            nonce: 0,
            balance: U256::from(0u64),
            storage_root: H256::from(EMPTY_ROOT),
            code_hash: H256::zero(),
        },
    );

    blockchain.commit(block1).expect("Failed to commit block 1");

    // Block 2: Transfer from Alice to Bob
    let block2_hash = H256::repeat_byte(0x02);
    let mut block2 = blockchain
        .start_new(block1_hash, block2_hash, 2)
        .expect("Failed to create block 2");

    // Get Alice's state from block 1
    let alice_acc = blockchain
        .get_account(&block1_hash, &alice)
        .expect("Alice should exist");

    // Transfer 100 from Alice to Bob
    block2.set_account(
        alice,
        Account {
            nonce: alice_acc.nonce + 1,
            balance: alice_acc.balance - U256::from(100u64),
            ..alice_acc
        },
    );

    let bob_acc = blockchain
        .get_account(&block1_hash, &bob)
        .expect("Bob should exist");
    block2.set_account(
        bob,
        Account {
            balance: bob_acc.balance + U256::from(100u64),
            ..bob_acc
        },
    );

    blockchain.commit(block2).expect("Failed to commit block 2");

    // Verify state in block 2
    let alice_final = blockchain
        .get_account(&block2_hash, &alice)
        .expect("Alice should exist in block 2");
    let bob_final = blockchain
        .get_account(&block2_hash, &bob)
        .expect("Bob should exist in block 2");

    assert_eq!(alice_final.balance, U256::from(900u64));
    assert_eq!(alice_final.nonce, 1);
    assert_eq!(bob_final.balance, U256::from(100u64));
}

/// Test that stale blocks are pruned after finalization via fork_choice_update.
/// This verifies the prune_stale_blocks() functionality.
#[test]
fn test_prune_stale_blocks_after_finalization() {
    let db = PagedDb::in_memory(1000).expect("Failed to create database");
    let blockchain = Blockchain::new(db);

    let genesis_hash = blockchain.last_finalized_hash();

    // Create a fork: block1 and block1_alt both have genesis as parent
    let block1_hash = H256::repeat_byte(0x01);
    let block1_alt_hash = H256::repeat_byte(0xA1);

    let block1 = blockchain
        .start_new(genesis_hash, block1_hash, 1)
        .expect("Failed to create block 1");
    let block1_alt = blockchain
        .start_new(genesis_hash, block1_alt_hash, 1)
        .expect("Failed to create block 1 alt");

    blockchain.commit(block1).expect("Failed to commit block 1");
    blockchain
        .commit(block1_alt)
        .expect("Failed to commit block 1 alt");

    // Both blocks should exist
    assert_eq!(blockchain.committed_count(), 2);
    assert!(blockchain.get_block(&block1_hash).is_some());
    assert!(blockchain.get_block(&block1_alt_hash).is_some());

    // Build further on block1 (the winning chain)
    let block2_hash = H256::repeat_byte(0x02);
    let block2 = blockchain
        .start_new(block1_hash, block2_hash, 2)
        .expect("Failed to create block 2");
    blockchain.commit(block2).expect("Failed to commit block 2");

    assert_eq!(blockchain.committed_count(), 3);

    // Finalize block1 via fork_choice_update (this calls prune_stale_blocks internally)
    blockchain
        .fork_choice_update(block2_hash, None, Some(block1_hash))
        .expect("Fork choice update should succeed");

    // Verify finalization
    assert_eq!(blockchain.last_finalized_number(), 1);
    assert_eq!(blockchain.last_finalized_hash(), block1_hash);

    // block1 is finalized (moved to cold storage), block1_alt should be pruned
    // Only block2 should remain in hot storage
    assert_eq!(blockchain.committed_count(), 1);

    // The stale block1_alt should no longer be retrievable
    assert!(blockchain.get_block(&block1_alt_hash).is_none());

    // block2 should still be in hot storage
    assert!(blockchain.get_block(&block2_hash).is_some());
}

/// Test that multiple stale forks are pruned after finalization.
#[test]
fn test_prune_multiple_stale_forks() {
    let db = PagedDb::in_memory(1000).expect("Failed to create database");
    let blockchain = Blockchain::new(db);

    let genesis_hash = blockchain.last_finalized_hash();

    // Create the main chain: block1 -> block2
    let block1_hash = H256::repeat_byte(0x01);
    let block2_hash = H256::repeat_byte(0x02);

    let block1 = blockchain
        .start_new(genesis_hash, block1_hash, 1)
        .expect("Failed to create block 1");
    blockchain.commit(block1).expect("Failed to commit block 1");

    let block2 = blockchain
        .start_new(block1_hash, block2_hash, 2)
        .expect("Failed to create block 2");
    blockchain.commit(block2).expect("Failed to commit block 2");

    // Create multiple forks at different heights
    // Fork at genesis level
    let fork_at_1a = H256::repeat_byte(0xA1);
    let fork_at_1b = H256::repeat_byte(0xB1);
    let fork1a = blockchain
        .start_new(genesis_hash, fork_at_1a, 1)
        .expect("Failed to create fork 1a");
    let fork1b = blockchain
        .start_new(genesis_hash, fork_at_1b, 1)
        .expect("Failed to create fork 1b");
    blockchain.commit(fork1a).expect("Failed to commit fork 1a");
    blockchain.commit(fork1b).expect("Failed to commit fork 1b");

    // Fork at block1 level
    let fork_at_2a = H256::repeat_byte(0xA2);
    let fork2a = blockchain
        .start_new(block1_hash, fork_at_2a, 2)
        .expect("Failed to create fork 2a");
    blockchain.commit(fork2a).expect("Failed to commit fork 2a");

    // Total: 5 blocks in hot storage
    assert_eq!(blockchain.committed_count(), 5);

    // Finalize block2 via fork_choice_update
    blockchain
        .fork_choice_update(block2_hash, None, Some(block2_hash))
        .expect("Fork choice update should succeed");

    // Verify finalization
    assert_eq!(blockchain.last_finalized_number(), 2);
    assert_eq!(blockchain.last_finalized_hash(), block2_hash);

    // All blocks at or below finalized height (except the finalized chain) should be pruned
    // The finalized block (block2) is moved to cold storage
    // No blocks should remain in hot storage
    assert_eq!(blockchain.committed_count(), 0);

    // Stale forks should be pruned
    assert!(blockchain.get_block(&fork_at_1a).is_none());
    assert!(blockchain.get_block(&fork_at_1b).is_none());
    assert!(blockchain.get_block(&fork_at_2a).is_none());
}
