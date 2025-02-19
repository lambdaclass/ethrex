#![no_main]

use ethrex_blockchain::{validate_block, validate_gas_used};
use ethrex_vm::{backends::revm_b::REVM, db::EvmState};
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
    let mut state = EvmState::from(db.clone());
    let chain_config = state
        .chain_config()
        .expect("Failed to get chain config from state");

    // Validate the block
    validate_block(&block, &parent_block_header, &chain_config).expect("invalid block");

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

    let (receipts, account_updates) =
        REVM::execute_block(&block, &mut state).expect("failed to execute block");
    validate_gas_used(&receipts, &block.header).expect("invalid gas used");

    // Output gas for measurement purposes
    let cumulative_gas_used = receipts
        .last()
        .map(|last_receipt| last_receipt.cumulative_gas_used)
        .unwrap_or_default();
    sp1_zkvm::io::commit(&cumulative_gas_used);

    // Update tries and calculate final state root hash
    update_tries(&mut state_trie, &mut storage_tries, &account_updates)
        .expect("failed to update state and storage tries");

    let final_state_hash = state_trie.hash_no_commit();
    if final_state_hash != block.header.state_root {
        panic!("invalid final state trie");
    }

    sp1_zkvm::io::commit(&ProgramOutput {
        initial_state_hash,
        final_state_hash,
    });
}
