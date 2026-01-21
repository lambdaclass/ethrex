//! Basic usage example for ethrex_db
//!
//! This example demonstrates the core API for managing Ethereum state.

use ethrex_db::merkle::{keccak256, EMPTY_ROOT};
use ethrex_db::store::{AccountData, StateTrie, StorageTrie};

fn main() {
    println!("=== ethrex_db Basic Usage ===\n");

    // 1. Create a new state trie
    let mut state = StateTrie::new();
    println!("Created empty state trie");
    println!("Empty root: {:?}\n", hex::encode(state.root_hash()));

    // 2. Create an account
    let address = [0x42u8; 20]; // Example address
    let account = AccountData {
        nonce: 1,
        balance: {
            let mut b = [0u8; 32];
            b[31] = 100; // 100 wei
            b
        },
        storage_root: EMPTY_ROOT,
        code_hash: AccountData::EMPTY_CODE_HASH,
    };

    state.set_account(&address, account);
    println!("Added account at 0x{}", hex::encode(address));

    // 3. Read the account back
    if let Some(retrieved) = state.get_account(&address) {
        println!("  Nonce: {}", retrieved.nonce);
        println!("  Balance: {} wei", retrieved.balance[31]);
    }

    // 4. Compute state root
    let root = state.root_hash();
    println!("\nState root: 0x{}", hex::encode(root));

    // 5. Demonstrate storage trie
    println!("\n=== Storage Trie ===\n");

    let mut storage = StorageTrie::new();
    println!("Created empty storage trie");
    println!(
        "Empty storage root: 0x{}\n",
        hex::encode(storage.root_hash())
    );

    // Set some storage values
    let slot_0 = [0u8; 32];
    let mut value_0 = [0u8; 32];
    value_0[31] = 42;

    storage.set(&slot_0, value_0);
    println!("Set slot 0 = 42");

    let slot_1 = {
        let mut s = [0u8; 32];
        s[31] = 1;
        s
    };
    let mut value_1 = [0u8; 32];
    value_1[31] = 100;

    storage.set(&slot_1, value_1);
    println!("Set slot 1 = 100");

    // Read storage
    if let Some(v) = storage.get(&slot_0) {
        println!("\nSlot 0 value: {}", v[31]);
    }
    if let Some(v) = storage.get(&slot_1) {
        println!("Slot 1 value: {}", v[31]);
    }

    println!("\nStorage root: 0x{}", hex::encode(storage.root_hash()));

    // 6. Demonstrate keccak256 hashing
    println!("\n=== Keccak256 Hashing ===\n");

    let data = b"hello world";
    let hash = keccak256(data);
    println!("keccak256('hello world') = 0x{}", hex::encode(hash));

    println!("\nDone!");
}
