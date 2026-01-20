//! Merkle trie example
//!
//! Demonstrates the Merkle Patricia Trie implementation for Ethereum state.

use ethrex_db::merkle::{MerkleTrie, keccak256, EMPTY_ROOT};

fn main() {
    println!("=== Merkle Patricia Trie Example ===\n");

    // 1. Create a new trie
    let mut trie = MerkleTrie::new();
    println!("Created empty Merkle trie");
    println!("Empty trie root: 0x{}", hex::encode(EMPTY_ROOT));
    println!("(This is keccak256 of RLP-encoded empty string)\n");

    // 2. Insert some key-value pairs
    // In Ethereum, keys are typically keccak256 hashes of addresses
    let key1 = keccak256(b"account_1");
    let value1 = b"Alice's data".to_vec();

    let key2 = keccak256(b"account_2");
    let value2 = b"Bob's data".to_vec();

    let key3 = keccak256(b"account_3");
    let value3 = b"Carol's data".to_vec();

    println!("Inserting 3 accounts...");
    trie.insert(&key1, value1);
    let root1 = trie.root_hash();
    println!("After account 1: 0x{}", hex::encode(root1));

    trie.insert(&key2, value2);
    let root2 = trie.root_hash();
    println!("After account 2: 0x{}", hex::encode(root2));

    trie.insert(&key3, value3);
    let root3 = trie.root_hash();
    println!("After account 3: 0x{}", hex::encode(root3));

    // 3. Read values back
    println!("\nReading values:");
    if let Some(v) = trie.get(&key1) {
        println!("account_1: {}", String::from_utf8_lossy(v));
    }
    if let Some(v) = trie.get(&key2) {
        println!("account_2: {}", String::from_utf8_lossy(v));
    }

    // 4. Update a value and see root change
    println!("\nUpdating account 1...");
    trie.insert(&key1, b"Alice's updated data".to_vec());
    let root4 = trie.root_hash();
    println!("New root: 0x{}", hex::encode(root4));
    assert_ne!(root3, root4, "Root should change after update");

    // 5. Delete a value
    println!("\nDeleting account 2...");
    trie.remove(&key2);
    let root5 = trie.root_hash();
    println!("After deletion: 0x{}", hex::encode(root5));

    // Verify it's gone
    assert!(trie.get(&key2).is_none(), "account_2 should be deleted");
    println!("Verified: account_2 no longer exists");

    // 6. Demonstrate determinism
    println!("\n=== Determinism Test ===");
    let mut trie_a = MerkleTrie::new();
    let mut trie_b = MerkleTrie::new();

    // Insert in different orders
    trie_a.insert(&key1, b"value1".to_vec());
    trie_a.insert(&key2, b"value2".to_vec());
    trie_a.insert(&key3, b"value3".to_vec());

    trie_b.insert(&key3, b"value3".to_vec()); // Different order
    trie_b.insert(&key1, b"value1".to_vec());
    trie_b.insert(&key2, b"value2".to_vec());

    let root_a = trie_a.root_hash();
    let root_b = trie_b.root_hash();

    println!("Trie A root: 0x{}", hex::encode(root_a));
    println!("Trie B root: 0x{}", hex::encode(root_b));
    assert_eq!(root_a, root_b, "Same data = same root regardless of insertion order");
    println!("Roots match - trie is deterministic!");

    // 7. Iterate over all entries
    println!("\n=== Iterating Trie Contents ===");
    for (key, value) in trie.iter() {
        println!("Key: 0x{}... Value len: {} bytes",
                 hex::encode(&key[..4]),
                 value.len());
    }

    println!("\nDone!");
}
