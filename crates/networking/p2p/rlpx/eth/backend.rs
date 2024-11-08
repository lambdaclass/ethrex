use ethereum_rust_core::{types::ForkId, U256};
use ethereum_rust_storage::Store;

use crate::rlpx::error::RLPxError;

use super::status::StatusMessage;

pub const ETH_VERSION: u32 = 68;

pub fn get_status(storage: &Store) -> Result<StatusMessage, RLPxError> {
    let chain_config = storage.get_chain_config()?;
    let total_difficulty = U256::from(chain_config.terminal_total_difficulty.unwrap_or_default());
    let network_id = chain_config.chain_id;

    // These blocks must always be available
    let genesis_header = storage
        .get_block_header(0)?
        .ok_or(RLPxError::NotFound("Genesis Block".to_string()))?;
    let block_number = storage
        .get_latest_block_number()?
        .ok_or(RLPxError::NotFound("Latest Block Number".to_string()))?;
    let block_header = storage
        .get_block_header(block_number)?
        .ok_or(RLPxError::NotFound(format!("Block {block_number}")))?;

    let genesis = genesis_header.compute_block_hash();
    let block_hash = block_header.compute_block_hash();
    let fork_id = ForkId::new(chain_config, genesis, block_header.timestamp, block_number);
    Ok(StatusMessage {
        eth_version: ETH_VERSION,
        network_id,
        total_difficulty,
        block_hash,
        genesis,
        fork_id,
    })
}
