use ethereum_rust_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use risc0_zkvm::guest::env;

use ethereum_rust_blockchain::{validate_block, validate_gas_used};
use ethereum_rust_core::types::{Block, BlockHeader};
use ethereum_rust_vm::{execute_block, execution_db::ExecutionDB, get_state_transitions, EvmState};

fn main() {
    let (block, execution_db, parent_header) = read_inputs().expect("failed to read inputs");
    let mut state = EvmState::from_exec_db(execution_db.clone());

    // Validate the block pre-execution
    validate_block(&block, &parent_header, &state).expect("invalid block");

    let receipts = execute_block(&block, &mut state).unwrap();

    env::commit(&receipts);

    validate_gas_used(&receipts, &block.header).expect("invalid gas used");

    let _account_updates = get_state_transitions(&mut state);

    // TODO: compute new state root from account updates and check it matches with the block's
    // header one.
}

fn read_inputs() -> Result<(Block, ExecutionDB, BlockHeader), RLPDecodeError> {
    let head_block_bytes = env::read::<Vec<u8>>();
    let execution_db = env::read::<ExecutionDB>();
    let parent_header_bytes = env::read::<Vec<u8>>();

    let block = Block::decode(&head_block_bytes)?;
    let parent_header = BlockHeader::decode(&parent_header_bytes)?;

    // make inputs public
    env::commit(&block.encode_to_vec());
    env::commit(&execution_db);
    env::commit(&parent_header.encode_to_vec());

    Ok((block, execution_db, parent_header))
}
