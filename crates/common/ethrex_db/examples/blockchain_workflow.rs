//! Blockchain workflow example
//!
//! Shows how to use the full ethrex_db stack: PagedDb + Blockchain + State Tries.

use ethrex_db::chain::{Account, Blockchain, ReadOnlyWorldState, WorldState};
use ethrex_db::merkle::EMPTY_ROOT;
use ethrex_db::store::PagedDb;
use primitive_types::{H256, U256};

fn main() {
    println!("=== Blockchain Workflow Example ===\n");

    // 1. Create an in-memory database
    let db = PagedDb::in_memory(1000).expect("Failed to create database");
    println!("Created in-memory PagedDb with 1000 pages (4MB)");

    // 2. Create a blockchain on top
    let blockchain = Blockchain::new(db);
    println!("Created Blockchain layer");
    println!(
        "Last finalized: {} (hash: {:?})\n",
        blockchain.last_finalized_number(),
        blockchain.last_finalized_hash()
    );

    // 3. Create a new block on top of finalized state
    let parent_hash = blockchain.last_finalized_hash();
    let block1_hash = H256::repeat_byte(0x01);
    let mut block1 = blockchain
        .start_new(parent_hash, block1_hash, 1)
        .expect("Failed to create block");
    println!("Created block 1 (hash: {:?})", block1_hash);

    // 4. Add accounts to the block
    let alice = H256::from_low_u64_be(1);
    let bob = H256::from_low_u64_be(2);
    let contract = H256::from_low_u64_be(0x1000);

    block1.set_account(
        alice,
        Account {
            nonce: 0,
            balance: U256::from(1_000_000_000u64), // 1 Gwei
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

    block1.set_account(
        contract,
        Account {
            nonce: 1,
            balance: U256::zero(),
            storage_root: H256::from(EMPTY_ROOT),
            code_hash: H256::from_low_u64_be(0xDEADBEEF),
        },
    );

    println!("Added accounts: Alice, Bob, and Contract");

    // Read back account data before commit
    if let Some(alice_acc) = block1.get_account(&alice) {
        println!("Alice balance: {} wei", alice_acc.balance);
    }

    // 5. Commit the block
    blockchain.commit(block1).expect("Failed to commit");
    println!(
        "Committed block 1 (now there are {} uncommitted blocks)",
        blockchain.committed_count()
    );

    // 6. Create a second block on top of block 1
    let block2_hash = H256::repeat_byte(0x02);
    let mut block2 = blockchain
        .start_new(block1_hash, block2_hash, 2)
        .expect("Failed to create block 2");
    println!("\nCreated block 2 (hash: {:?})", block2_hash);

    // 7. Simulate a transaction (Alice sends to Bob)
    // Note: We need to get account from blockchain, not from block
    if let Some(alice_acc) = blockchain.get_account(&block1_hash, &alice) {
        println!("Alice's balance before tx: {} wei", alice_acc.balance);

        // Transfer 100M wei
        let transfer = U256::from(100_000_000u64);

        block2.set_account(
            alice,
            Account {
                nonce: alice_acc.nonce + 1,
                balance: alice_acc.balance - transfer,
                ..alice_acc
            },
        );

        if let Some(bob_acc) = blockchain.get_account(&block1_hash, &bob) {
            block2.set_account(
                bob,
                Account {
                    balance: bob_acc.balance + transfer,
                    ..bob_acc
                },
            );
        }

        println!("Transferred 100M wei from Alice to Bob");
    }

    blockchain.commit(block2).expect("Failed to commit block 2");
    println!("Committed block 2");

    // 8. Create parallel blocks (for handling potential reorgs)
    println!("\n=== Parallel Block Creation ===");

    let block2_alt_hash = H256::repeat_byte(0x22);
    let mut block2_alt = blockchain
        .start_new(block1_hash, block2_alt_hash, 2)
        .expect("Failed to create alternative block 2");
    println!("Created alternative block 2 (hash: {:?})", block2_alt_hash);

    // Different transaction in this branch
    if let Some(alice_acc) = blockchain.get_account(&block1_hash, &alice) {
        block2_alt.set_account(
            alice,
            Account {
                nonce: alice_acc.nonce + 1,
                balance: alice_acc.balance - U256::from(50_000_000u64),
                ..alice_acc
            },
        );
        println!("In alternative branch: Alice sent 50M wei elsewhere");
    }

    blockchain
        .commit(block2_alt)
        .expect("Failed to commit alt block");
    println!(
        "Now have {} committed blocks (both branches exist)",
        blockchain.committed_count()
    );

    // 9. Fork choice update - choose the original block2
    blockchain
        .fork_choice_update(block2_hash, None, None)
        .expect("Fork choice failed");
    println!("\nFork choice selected block 2 as head");

    // 10. Finalize block 1
    println!("\n=== Finalization ===");
    blockchain
        .finalize(block1_hash)
        .expect("Failed to finalize");
    println!("Finalized block 1");
    println!(
        "Last finalized: {} (hash: {:?})",
        blockchain.last_finalized_number(),
        blockchain.last_finalized_hash()
    );
    println!(
        "Committed blocks remaining: {}",
        blockchain.committed_count()
    );

    println!("\nDone!");
}
