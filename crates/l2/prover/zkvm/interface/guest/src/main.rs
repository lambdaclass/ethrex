use risc0_zkvm::guest::env;

use ethrex_blockchain::{validate_block, validate_gas_used};
use ethrex_vm::{execute_block, get_state_transitions, EvmState};
use zkvm_interface::{
    io::{ProgramInput, ProgramOutput},
    trie::update_tries,
};

fn main() {
    let ProgramInput {
        block,
        parent_block_header,
        db,
    } = env::read();
    let mut state = EvmState::from(db.clone());

    // Validate the block pre-execution
    validate_block(&block, &parent_block_header, &state).expect("invalid block");

    // Validate the initial state
    let (mut state_trie, mut storage_tries) = db
        .build_tries()
        .expect("failed to build state and storage tries or state is not valid");

    let initial_state_hash = state_trie.hash_no_commit();
    if initial_state_hash != parent_block_header.state_root {
        panic!("invalid initial state trie");
    }

    let receipts = execute_block(&block, &mut state).expect("failed to execute block");
    validate_gas_used(&receipts, &block.header).expect("invalid gas used");

    let account_updates = get_state_transitions(&mut state);

    // Update tries and calculate final state root hash
    update_tries(&mut state_trie, &mut storage_tries, &account_updates)
        .expect("failed to update state and storage tries");
    let final_state_hash = state_trie.hash_no_commit();

    if final_state_hash != block.header.state_root {
        panic!("invalid final state trie");
    }

    env::commit(&ProgramOutput {
        initial_state_hash,
        final_state_hash,
    });
}
