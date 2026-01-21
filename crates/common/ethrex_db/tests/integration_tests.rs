//! Integration tests for ethrex_db.

use ethrex_db::chain::{Account, Blockchain, WorldState};
use ethrex_db::data::{NibblePath, SlottedArray};
use ethrex_db::merkle::{keccak256, MerkleTrie, EMPTY_ROOT};
use ethrex_db::store::{CommitOptions, PageType, PagedDb};
use primitive_types::{H256, U256};

#[test]
fn test_full_workflow() {
    // Create database
    let db = PagedDb::in_memory(1000).unwrap();

    // Create blockchain
    let blockchain = Blockchain::new(db);

    // Create a block
    let parent_hash = blockchain.last_finalized_hash();
    let mut block = blockchain
        .start_new(parent_hash, H256::repeat_byte(0x01), 1)
        .unwrap();

    // Add account changes
    let address = H256::repeat_byte(0xAB);
    let account = Account::with_balance(U256::from(1000));
    block.set_account(address, account);

    // Add storage changes
    let storage_key = H256::repeat_byte(0xCD);
    block.set_storage(address, storage_key, U256::from(42));

    // Commit block
    blockchain.commit(block).unwrap();

    // Verify block is committed
    assert_eq!(blockchain.committed_count(), 1);
}

#[test]
fn test_merkle_trie_with_account_data() {
    let mut trie = MerkleTrie::new();

    // Simulate account state
    let accounts = vec![
        (
            H256::repeat_byte(0x01),
            Account::with_balance(U256::from(100)),
        ),
        (
            H256::repeat_byte(0x02),
            Account::with_balance(U256::from(200)),
        ),
        (
            H256::repeat_byte(0x03),
            Account::with_balance(U256::from(300)),
        ),
    ];

    for (address, account) in &accounts {
        let key = address.as_bytes();
        let value = account.encode();
        trie.insert(key, value);
    }

    let root = trie.root_hash();
    assert_ne!(root, EMPTY_ROOT);

    // Verify we can retrieve accounts
    for (address, account) in &accounts {
        let value = trie.get(address.as_bytes()).unwrap();
        let decoded = Account::decode(value).unwrap();
        assert_eq!(decoded.balance, account.balance);
    }
}

#[test]
fn test_slotted_array_stress() {
    let mut arr = SlottedArray::new();

    // Insert many entries
    let mut inserted = Vec::new();
    for i in 0..50 {
        let key_bytes = [i as u8, (i * 2) as u8];
        let key = NibblePath::from_bytes(&key_bytes);
        let value = format!("value_{}", i);

        if arr.try_insert(&key, value.as_bytes()) {
            inserted.push((key_bytes.to_vec(), value));
        } else {
            break; // Page is full
        }
    }

    // Verify all inserted entries
    for (key_bytes, expected_value) in &inserted {
        let key = NibblePath::from_bytes(key_bytes);
        let value = arr.get(&key).expect("Entry should exist");
        assert_eq!(value, expected_value.as_bytes());
    }
}

#[test]
fn test_database_persistence_simulation() {
    // This simulates what would happen with persistence

    // Create initial state
    let mut db = PagedDb::in_memory(1000).unwrap();

    // First batch
    {
        let mut batch = db.begin_batch();
        let (_addr, _page) = batch.allocate_page(PageType::Data, 0).unwrap();
        batch.set_metadata(1, &[0x11; 32]);
        batch.commit(CommitOptions::DangerNoFlush).unwrap();
    }

    assert_eq!(db.block_number(), 1);
    assert_eq!(db.block_hash(), [0x11; 32]);

    // Second batch
    {
        let mut batch = db.begin_batch();
        batch.set_metadata(2, &[0x22; 32]);
        batch.commit(CommitOptions::DangerNoFlush).unwrap();
    }

    assert_eq!(db.block_number(), 2);
    assert_eq!(db.block_hash(), [0x22; 32]);
}

#[test]
fn test_parallel_blocks() {
    let db = PagedDb::in_memory(1000).unwrap();
    let blockchain = Blockchain::new(db);

    let parent_hash = blockchain.last_finalized_hash();

    // Create multiple blocks from same parent (simulating parallel execution)
    let mut blocks = Vec::new();
    for i in 0..3 {
        let block = blockchain
            .start_new(parent_hash, H256::from_low_u64_be(i as u64 + 1), 1)
            .unwrap();
        blocks.push(block);
    }

    // Commit all blocks
    for block in blocks {
        blockchain.commit(block).unwrap();
    }

    // All blocks should be tracked
    assert_eq!(blockchain.committed_count(), 3);
}

#[test]
fn test_trie_branch_node_creation() {
    let mut trie = MerkleTrie::new();

    // Insert keys that will create a branch node
    // Keys starting with same prefix but different next nibble
    trie.insert(&[0x10], b"a".to_vec());
    trie.insert(&[0x11], b"b".to_vec());
    trie.insert(&[0x12], b"c".to_vec());
    trie.insert(&[0x20], b"d".to_vec());

    assert_eq!(trie.len(), 4);

    let root = trie.root_hash();
    assert_ne!(root, EMPTY_ROOT);

    // Verify all entries
    assert_eq!(trie.get(&[0x10]), Some(b"a".as_slice()));
    assert_eq!(trie.get(&[0x11]), Some(b"b".as_slice()));
    assert_eq!(trie.get(&[0x12]), Some(b"c".as_slice()));
    assert_eq!(trie.get(&[0x20]), Some(b"d".as_slice()));
}

#[test]
fn test_keccak256() {
    // Test against known Ethereum values
    let empty = keccak256(&[]);
    assert_eq!(
        hex::encode(empty),
        "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
    );

    let hello = keccak256(b"hello");
    assert_eq!(
        hex::encode(hello),
        "1c8aff950685c2ed4bc3174f3472287b56d9517b9c948127319a09a7a36deac8"
    );
}

#[test]
fn test_nibble_path_operations() {
    let path = NibblePath::from_bytes(&[0xAB, 0xCD, 0xEF]);

    // Test basic properties
    assert_eq!(path.len(), 6);
    assert_eq!(path.get(0), 0xA);
    assert_eq!(path.get(1), 0xB);
    assert_eq!(path.get(2), 0xC);
    assert_eq!(path.get(3), 0xD);
    assert_eq!(path.get(4), 0xE);
    assert_eq!(path.get(5), 0xF);

    // Test slicing
    let sliced = path.slice_from(2);
    assert_eq!(sliced.len(), 4);
    assert_eq!(sliced.get(0), 0xC);

    // Test common prefix
    let path2 = NibblePath::from_bytes(&[0xAB, 0x00]);
    assert_eq!(path.common_prefix_len(&path2), 2);
}

// ============================================================================
// End-to-End Tests
// ============================================================================

#[test]
fn test_e2e_multi_block_chain() {
    // Simulate building a chain of multiple blocks
    let db = PagedDb::in_memory(1000).unwrap();
    let blockchain = Blockchain::new(db);

    let mut prev_hash = blockchain.last_finalized_hash();

    // Build a chain of 10 blocks
    for block_num in 1..=10 {
        let block_hash = H256::from_low_u64_be(block_num as u64);
        let mut block = blockchain
            .start_new(prev_hash, block_hash, block_num)
            .unwrap();

        // Add some account activity in each block
        let address = H256::from_low_u64_be(block_num as u64 * 1000);
        let account = Account::with_balance(U256::from(block_num * 100));
        block.set_account(address, account);

        blockchain.commit(block).unwrap();
        prev_hash = block_hash;
    }

    assert_eq!(blockchain.committed_count(), 10);
}

#[test]
fn test_e2e_account_balance_transfers() {
    // Simulate balance transfers between accounts
    let db = PagedDb::in_memory(1000).unwrap();
    let blockchain = Blockchain::new(db);

    let alice = H256::repeat_byte(0xAA);
    let bob = H256::repeat_byte(0xBB);
    let charlie = H256::repeat_byte(0xCC);

    // Block 1: Initialize accounts
    let mut block1 = blockchain
        .start_new(
            blockchain.last_finalized_hash(),
            H256::from_low_u64_be(1),
            1,
        )
        .unwrap();

    block1.set_account(alice, Account::with_balance(U256::from(1000)));
    block1.set_account(bob, Account::with_balance(U256::from(500)));
    block1.set_account(charlie, Account::with_balance(U256::from(0)));

    let block1_hash = H256::from_low_u64_be(1);
    blockchain.commit(block1).unwrap();

    // Block 2: Alice sends 200 to Bob
    let mut block2 = blockchain
        .start_new(block1_hash, H256::from_low_u64_be(2), 2)
        .unwrap();
    block2.set_account(alice, Account::with_balance(U256::from(800)));
    block2.set_account(bob, Account::with_balance(U256::from(700)));
    blockchain.commit(block2).unwrap();

    // Block 3: Bob sends 100 to Charlie
    let mut block3 = blockchain
        .start_new(H256::from_low_u64_be(2), H256::from_low_u64_be(3), 3)
        .unwrap();
    block3.set_account(bob, Account::with_balance(U256::from(600)));
    block3.set_account(charlie, Account::with_balance(U256::from(100)));
    blockchain.commit(block3).unwrap();

    assert_eq!(blockchain.committed_count(), 3);
}

#[test]
fn test_e2e_contract_storage_operations() {
    // Simulate contract storage read/write operations
    let db = PagedDb::in_memory(1000).unwrap();
    let blockchain = Blockchain::new(db);

    let contract_addr = H256::repeat_byte(0xCC);

    // Deploy contract with initial storage
    let mut block1 = blockchain
        .start_new(
            blockchain.last_finalized_hash(),
            H256::from_low_u64_be(1),
            1,
        )
        .unwrap();

    // Contract account
    let mut contract = Account::with_balance(U256::zero());
    contract.nonce = 1;
    block1.set_account(contract_addr, contract.clone());

    // Initial storage slots
    for i in 0..10 {
        let slot = H256::from_low_u64_be(i);
        let value = U256::from(i * 100);
        block1.set_storage(contract_addr, slot, value);
    }

    blockchain.commit(block1).unwrap();

    // Update some storage slots
    let mut block2 = blockchain
        .start_new(H256::from_low_u64_be(1), H256::from_low_u64_be(2), 2)
        .unwrap();

    // Update slots 0, 5, and 9
    block2.set_storage(contract_addr, H256::from_low_u64_be(0), U256::from(999));
    block2.set_storage(contract_addr, H256::from_low_u64_be(5), U256::from(555));
    block2.set_storage(contract_addr, H256::from_low_u64_be(9), U256::from(0)); // Clear slot

    blockchain.commit(block2).unwrap();

    assert_eq!(blockchain.committed_count(), 2);
}

#[test]
fn test_e2e_large_state_trie() {
    // Test with a large number of accounts
    let mut trie = MerkleTrie::new();

    // Insert 1000 accounts
    for i in 0u64..1000 {
        let key = keccak256(&i.to_be_bytes());
        let account = Account::with_balance(U256::from(i * 1000));
        trie.insert(&key, account.encode());
    }

    let root1 = trie.root_hash();
    assert_ne!(root1, EMPTY_ROOT);
    assert_eq!(trie.len(), 1000);

    // Verify random accounts
    for i in [0u64, 100, 500, 999] {
        let key = keccak256(&i.to_be_bytes());
        let value = trie.get(&key).expect("Account should exist");
        let account = Account::decode(value).unwrap();
        assert_eq!(account.balance, U256::from(i * 1000));
    }

    // Update some accounts
    for i in 0u64..100 {
        let key = keccak256(&i.to_be_bytes());
        let account = Account::with_balance(U256::from(i * 2000));
        trie.insert(&key, account.encode());
    }

    let root2 = trie.root_hash();
    assert_ne!(root2, root1); // Root should change

    // Delete some accounts
    for i in 900u64..1000 {
        let key = keccak256(&i.to_be_bytes());
        trie.remove(&key);
    }

    assert_eq!(trie.len(), 900);
    let root3 = trie.root_hash();
    assert_ne!(root3, root2);
}

#[test]
fn test_e2e_state_root_consistency() {
    // Verify state root is deterministic regardless of insertion order
    let accounts: Vec<_> = (0u64..100)
        .map(|i| {
            let key = keccak256(&i.to_be_bytes());
            let account = Account::with_balance(U256::from(i * 100));
            (key, account.encode())
        })
        .collect();

    // Insert in forward order
    let mut trie1 = MerkleTrie::new();
    for (key, value) in accounts.iter() {
        trie1.insert(key, value.clone());
    }

    // Insert in reverse order
    let mut trie2 = MerkleTrie::new();
    for (key, value) in accounts.iter().rev() {
        trie2.insert(key, value.clone());
    }

    // Insert in random order (deterministic shuffle)
    let mut trie3 = MerkleTrie::new();
    let mut shuffled = accounts.clone();
    // Simple deterministic shuffle
    for i in 0..shuffled.len() {
        let j = (i * 7 + 13) % shuffled.len();
        shuffled.swap(i, j);
    }
    for (key, value) in shuffled.iter() {
        trie3.insert(key, value.clone());
    }

    // All tries should have the same root
    assert_eq!(trie1.root_hash(), trie2.root_hash());
    assert_eq!(trie2.root_hash(), trie3.root_hash());
}

#[test]
fn test_e2e_fork_choice_simulation() {
    // Simulate a fork choice scenario with competing blocks
    let db = PagedDb::in_memory(1000).unwrap();
    let blockchain = Blockchain::new(db);

    let genesis_hash = blockchain.last_finalized_hash();

    // Block 1 (canonical)
    let mut block1 = blockchain
        .start_new(genesis_hash, H256::from_low_u64_be(1), 1)
        .unwrap();
    block1.set_account(
        H256::repeat_byte(0x01),
        Account::with_balance(U256::from(100)),
    );
    blockchain.commit(block1).unwrap();

    // Block 2A and 2B (competing blocks from block 1)
    let block1_hash = H256::from_low_u64_be(1);

    let mut block2a = blockchain
        .start_new(block1_hash, H256::from_low_u64_be(0x2A), 2)
        .unwrap();
    block2a.set_account(
        H256::repeat_byte(0x02),
        Account::with_balance(U256::from(200)),
    );
    blockchain.commit(block2a).unwrap();

    let mut block2b = blockchain
        .start_new(block1_hash, H256::from_low_u64_be(0x2B), 2)
        .unwrap();
    block2b.set_account(
        H256::repeat_byte(0x02),
        Account::with_balance(U256::from(300)),
    );
    blockchain.commit(block2b).unwrap();

    // Block 3A (extends 2A)
    let mut block3a = blockchain
        .start_new(H256::from_low_u64_be(0x2A), H256::from_low_u64_be(0x3A), 3)
        .unwrap();
    block3a.set_account(
        H256::repeat_byte(0x03),
        Account::with_balance(U256::from(400)),
    );
    blockchain.commit(block3a).unwrap();

    // We now have a fork: genesis -> 1 -> 2A -> 3A
    //                                  \-> 2B
    assert_eq!(blockchain.committed_count(), 4);
}

#[test]
fn test_e2e_trie_proof_data() {
    // Test that we can iterate and verify all trie contents
    let mut trie = MerkleTrie::new();

    // Insert data
    let entries: Vec<_> = (0u64..50)
        .map(|i| {
            let key = keccak256(&format!("key_{}", i).as_bytes());
            let value = format!("value_{}", i).into_bytes();
            (key, value)
        })
        .collect();

    for (key, value) in &entries {
        trie.insert(key, value.clone());
    }

    let root = trie.root_hash();

    // Collect all entries via iteration
    let iter_entries: Vec<_> = trie.iter().map(|(k, v)| (k.to_vec(), v.to_vec())).collect();

    assert_eq!(iter_entries.len(), 50);

    // Verify each entry
    for (key, value) in &entries {
        assert!(trie.get(key).is_some());
        assert_eq!(trie.get(key).unwrap(), value.as_slice());
    }

    // Remove half and verify root changes
    for (key, _) in entries.iter().take(25) {
        trie.remove(key);
    }

    let new_root = trie.root_hash();
    assert_ne!(root, new_root);
    assert_eq!(trie.len(), 25);
}

#[test]
fn test_e2e_account_nonce_tracking() {
    // Test nonce increments for transaction simulation
    let db = PagedDb::in_memory(1000).unwrap();
    let blockchain = Blockchain::new(db);

    let sender = H256::repeat_byte(0xAA);

    // Initial account state
    let mut block1 = blockchain
        .start_new(
            blockchain.last_finalized_hash(),
            H256::from_low_u64_be(1),
            1,
        )
        .unwrap();

    let mut account = Account::with_balance(U256::from(10000));
    account.nonce = 0;
    block1.set_account(sender, account);
    blockchain.commit(block1).unwrap();

    // Simulate 5 transactions in block 2
    let mut block2 = blockchain
        .start_new(H256::from_low_u64_be(1), H256::from_low_u64_be(2), 2)
        .unwrap();

    let mut account = Account::with_balance(U256::from(9500)); // Spent some
    account.nonce = 5; // 5 transactions
    block2.set_account(sender, account);
    blockchain.commit(block2).unwrap();

    // Simulate 3 more transactions in block 3
    let mut block3 = blockchain
        .start_new(H256::from_low_u64_be(2), H256::from_low_u64_be(3), 3)
        .unwrap();

    let mut account = Account::with_balance(U256::from(9200));
    account.nonce = 8; // 5 + 3 transactions
    block3.set_account(sender, account);
    blockchain.commit(block3).unwrap();

    assert_eq!(blockchain.committed_count(), 3);
}

#[test]
fn test_e2e_empty_blocks() {
    // Test creating blocks with no state changes
    let db = PagedDb::in_memory(1000).unwrap();
    let blockchain = Blockchain::new(db);

    let mut prev_hash = blockchain.last_finalized_hash();

    // Create 5 empty blocks
    for i in 1..=5 {
        let block_hash = H256::from_low_u64_be(i as u64);
        let block = blockchain.start_new(prev_hash, block_hash, i).unwrap();
        blockchain.commit(block).unwrap();
        prev_hash = block_hash;
    }

    assert_eq!(blockchain.committed_count(), 5);
}

#[test]
fn test_e2e_deep_storage_trie() {
    // Test storage trie with many slots per contract
    let mut trie = MerkleTrie::new();

    // Simulate a contract with 500 storage slots
    for slot in 0u64..500 {
        let key = keccak256(&slot.to_be_bytes());
        let value = slot.to_be_bytes().to_vec();
        trie.insert(&key, value);
    }

    assert_eq!(trie.len(), 500);
    let root1 = trie.root_hash();

    // Verify all slots
    for slot in 0u64..500 {
        let key = keccak256(&slot.to_be_bytes());
        let value = trie.get(&key).expect("Slot should exist");
        assert_eq!(value, slot.to_be_bytes().as_slice());
    }

    // Update every 10th slot
    for slot in (0u64..500).step_by(10) {
        let key = keccak256(&slot.to_be_bytes());
        let new_value = (slot * 2).to_be_bytes().to_vec();
        trie.insert(&key, new_value);
    }

    let root2 = trie.root_hash();
    assert_ne!(root1, root2);

    // Clear every 5th slot
    for slot in (0u64..500).step_by(5) {
        let key = keccak256(&slot.to_be_bytes());
        trie.remove(&key);
    }

    assert_eq!(trie.len(), 400); // 500 - 100 = 400
}

#[test]
fn test_e2e_mixed_key_lengths() {
    // Test trie with various key lengths
    let mut trie = MerkleTrie::new();

    // Short keys
    trie.insert(&[0x01], b"short1".to_vec());
    trie.insert(&[0x02], b"short2".to_vec());

    // Medium keys
    trie.insert(&[0x01, 0x02, 0x03, 0x04], b"medium1".to_vec());
    trie.insert(&[0x01, 0x02, 0x03, 0x05], b"medium2".to_vec());

    // Long keys (32 bytes like Ethereum addresses)
    let long_key1 = keccak256(b"long_key_1");
    let long_key2 = keccak256(b"long_key_2");
    trie.insert(&long_key1, b"long1".to_vec());
    trie.insert(&long_key2, b"long2".to_vec());

    assert_eq!(trie.len(), 6);

    // Verify all
    assert_eq!(trie.get(&[0x01]), Some(b"short1".as_slice()));
    assert_eq!(
        trie.get(&[0x01, 0x02, 0x03, 0x04]),
        Some(b"medium1".as_slice())
    );
    assert_eq!(trie.get(&long_key1), Some(b"long1".as_slice()));

    // Remove and verify
    trie.remove(&[0x01]);
    assert_eq!(trie.get(&[0x01]), None);
    assert_eq!(trie.len(), 5);
}

#[test]
fn test_e2e_batch_operations() {
    // Test batch page operations
    let mut db = PagedDb::in_memory(1000).unwrap();

    // Allocate multiple pages in one batch
    {
        let mut batch = db.begin_batch();

        for i in 0..10 {
            let (addr, _page) = batch.allocate_page(PageType::Data, i as u8).unwrap();
            assert!(!addr.is_null());
        }

        batch.set_metadata(1, &[0xAA; 32]);
        batch.commit(CommitOptions::DangerNoFlush).unwrap();
    }

    assert_eq!(db.block_number(), 1);

    // Allocate more in a second batch
    {
        let mut batch = db.begin_batch();

        for i in 10..20 {
            let (addr, _page) = batch.allocate_page(PageType::Data, i as u8).unwrap();
            assert!(!addr.is_null());
        }

        batch.set_metadata(2, &[0xBB; 32]);
        batch.commit(CommitOptions::DangerNoFlush).unwrap();
    }

    assert_eq!(db.block_number(), 2);
    assert_eq!(db.block_hash(), [0xBB; 32]);
}

#[test]
fn test_e2e_trie_edge_cases() {
    let mut trie = MerkleTrie::new();

    // Very long value
    let long_value = vec![0xAB; 1000];
    trie.insert(b"long", long_value.clone());
    assert_eq!(trie.get(b"long"), Some(long_value.as_slice()));

    // Key that's all zeros
    let zero_key = [0u8; 32];
    trie.insert(&zero_key, b"zero_key".to_vec());
    assert_eq!(trie.get(&zero_key), Some(b"zero_key".as_slice()));

    // Key that's all ones
    let ones_key = [0xFF; 32];
    trie.insert(&ones_key, b"ones_key".to_vec());
    assert_eq!(trie.get(&ones_key), Some(b"ones_key".as_slice()));

    // Single byte value
    trie.insert(b"single", vec![0x42]);
    assert_eq!(trie.get(b"single"), Some([0x42].as_slice()));

    // Verify count
    assert_eq!(trie.len(), 4);

    // Remove all
    trie.remove(b"long");
    trie.remove(&zero_key);
    trie.remove(&ones_key);
    trie.remove(b"single");

    assert!(trie.is_empty());
    assert_eq!(trie.root_hash(), EMPTY_ROOT);
}

#[test]
fn test_e2e_state_persistence() {
    // Test full state persistence: create blocks, finalize, and verify state root
    let db = PagedDb::in_memory(1000).unwrap();
    let blockchain = Blockchain::new(db);

    // Create and commit several blocks with state changes
    let genesis_hash = blockchain.last_finalized_hash();

    // Block 1: Create two accounts
    let mut block1 = blockchain
        .start_new(genesis_hash, H256::from_low_u64_be(1), 1)
        .unwrap();
    let addr1 = H256::repeat_byte(0x01);
    let addr2 = H256::repeat_byte(0x02);
    block1.set_account(addr1, Account::with_balance(U256::from(1000)));
    block1.set_account(addr2, Account::with_balance(U256::from(2000)));
    blockchain.commit(block1).unwrap();

    // Block 2: Update account 1, add storage to account 2
    let mut block2 = blockchain
        .start_new(H256::from_low_u64_be(1), H256::from_low_u64_be(2), 2)
        .unwrap();
    block2.set_account(addr1, Account::with_balance(U256::from(900)));
    block2.set_storage(addr2, H256::repeat_byte(0xAA), U256::from(42));
    blockchain.commit(block2).unwrap();

    // Block 3: More storage changes
    let mut block3 = blockchain
        .start_new(H256::from_low_u64_be(2), H256::from_low_u64_be(3), 3)
        .unwrap();
    block3.set_storage(addr2, H256::repeat_byte(0xBB), U256::from(100));
    block3.set_storage(addr2, H256::repeat_byte(0xCC), U256::from(200));
    blockchain.commit(block3).unwrap();

    // Before finalization, state root should be empty (no finalized state)
    let initial_root = blockchain.state_root();
    assert_eq!(initial_root, EMPTY_ROOT);

    // Finalize block 3 (which includes blocks 1, 2, and 3)
    blockchain.finalize(H256::from_low_u64_be(3)).unwrap();

    // After finalization, state root should change
    let final_root = blockchain.state_root();
    assert_ne!(final_root, EMPTY_ROOT);

    // Query finalized state
    let addr1_bytes: [u8; 20] = addr1.as_bytes()[12..32].try_into().unwrap();
    let account1 = blockchain.get_finalized_account(&addr1_bytes);
    assert!(account1.is_some());
    assert_eq!(account1.unwrap().balance, U256::from(900));

    // Verify finalization state
    assert_eq!(blockchain.last_finalized_number(), 3);
    assert_eq!(blockchain.committed_count(), 0);
}

#[test]
fn test_e2e_incremental_finalization() {
    // Test incremental finalization (finalize block by block)
    let db = PagedDb::in_memory(1000).unwrap();
    let blockchain = Blockchain::new(db);

    let genesis_hash = blockchain.last_finalized_hash();

    // Create 5 blocks
    let mut prev_hash = genesis_hash;
    for i in 1..=5 {
        let mut block = blockchain
            .start_new(prev_hash, H256::from_low_u64_be(i), i)
            .unwrap();
        let addr = H256::from_low_u64_be(i * 100);
        block.set_account(addr, Account::with_balance(U256::from(i * 1000)));
        blockchain.commit(block).unwrap();
        prev_hash = H256::from_low_u64_be(i);
    }

    // Finalize block 2
    blockchain.finalize(H256::from_low_u64_be(2)).unwrap();
    assert_eq!(blockchain.last_finalized_number(), 2);
    assert_eq!(blockchain.committed_count(), 3); // blocks 3, 4, 5 remain

    let root_after_2 = blockchain.state_root();
    assert_ne!(root_after_2, EMPTY_ROOT);

    // Finalize block 4
    blockchain.finalize(H256::from_low_u64_be(4)).unwrap();
    assert_eq!(blockchain.last_finalized_number(), 4);
    assert_eq!(blockchain.committed_count(), 1); // block 5 remains

    let root_after_4 = blockchain.state_root();
    assert_ne!(root_after_4, root_after_2); // Root should change with more state

    // Finalize block 5
    blockchain.finalize(H256::from_low_u64_be(5)).unwrap();
    assert_eq!(blockchain.last_finalized_number(), 5);
    assert_eq!(blockchain.committed_count(), 0);

    let root_after_5 = blockchain.state_root();
    assert_ne!(root_after_5, root_after_4);
}
