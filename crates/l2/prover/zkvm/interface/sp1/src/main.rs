#![no_main]

use ethrex_blockchain::{validate_block, validate_gas_used};
use ethrex_vm::Evm;
use zkvm_interface::{
    io::{ProgramInput, ProgramOutput},
    trie::{update_tries, verify_db},
};

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let ProgramInput {
        block,
        parent_block_header,
        db,
    } = sp1_zkvm::io::read::<ProgramInput>();
    // Validate the block
    validate_block(&block, &parent_block_header, &db.chain_config).expect("invalid block");

    // Tries used for validating initial and final state root
    let (mut state_trie, mut storage_tries) = db
        .get_tries()
        .expect("failed to build state and storage tries or state is not valid");

    // Validate the initial state
    let initial_state_hash = state_trie.hash_no_commit();
    if initial_state_hash != parent_block_header.state_root {
        panic!("invalid initial state trie");
    }
    if !verify_db(&db, &state_trie, &storage_tries).expect("failed to validate database") {
        panic!("invalid database")
    };
    let fork = db.chain_config.fork(block.header.timestamp);

    let mut evm = Evm::from_execution_db(db.clone());
    let result = evm.execute_block(&block).expect("failed to execute block");
    let receipts = result.receipts;
    let account_updates = evm.get_state_transitions(fork).expect("failed to get state transitions");
    // validate_gas_used(&receipts, &block.header).expect("invalid gas used");

    // Output gas for measurement purposes
    let cumulative_gas_used = receipts
        .last()
        .map(|last_receipt| last_receipt.cumulative_gas_used)
        .unwrap_or_default();
    sp1_zkvm::io::commit(&cumulative_gas_used);

    // Update state trie
    update_tries(&mut state_trie, &mut storage_tries, &account_updates)
        .expect("failed to update state and storage tries");

    // Calculate final state root hash and check
    let final_state_hash = state_trie.hash_no_commit();
    // if final_state_hash != block.header.state_root {
    //     panic!("invalid final state trie");
    // }

    sp1_zkvm::io::commit(&ProgramOutput {
        initial_state_hash,
        final_state_hash,
    });
}
