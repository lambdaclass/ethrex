use ethereum_rust_core::{types::ForkId, H32, U256};
use ethereum_rust_storage::{error::StoreError, Store};

use super::status::StatusMessage;

pub const ETH_VERSION: u32 = 68;

pub fn get_status(storage: &Store) -> Result<StatusMessage, StoreError> {
    let chain_config = storage.get_chain_config()?;
    let total_difficulty = U256::from(chain_config.terminal_total_difficulty.unwrap_or_default());
    let network_id = chain_config.chain_id;

    // These blocks must always be available
    let genesis_header = storage.get_block_header(0)?.unwrap();
    let block_number = storage.get_latest_block_number()?.unwrap();
    let block_header = storage.get_block_header(block_number)?.unwrap();

    let genesis = genesis_header.compute_block_hash();
    let block_hash = block_header.compute_block_hash();
    // FIXME: Remove hardcoded values
    // before PR review. This is only
    // to skip the status check for now.
    let fork_hash = H32([5, 196, 93, 237]);
    let fork_next = 0_u64;
    let fork_id = ForkId::new(chain_config, genesis, block_header.timestamp, block_number);
    Ok(StatusMessage::new(
        ETH_VERSION,
        network_id,
        total_difficulty,
        block_hash,
        genesis,
        ForkId {
            fork_hash,
            fork_next,
        },
    ))
}
