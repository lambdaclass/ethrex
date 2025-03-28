use ethrex_common::{types::ForkId, U256};
use ethrex_storage::Store;

use crate::rlpx::error::RLPxError;

use super::status::StatusMessage;

pub fn get_status(storage: &Store, eth_version: u32) -> Result<StatusMessage, RLPxError> {
    let chain_config = storage.get_chain_config()?;
    let total_difficulty = U256::from(chain_config.terminal_total_difficulty.unwrap_or_default());
    let network_id = chain_config.chain_id;

    // These blocks must always be available
    let genesis_header = storage
        .get_block_header(0)?
        .ok_or(RLPxError::NotFound("Genesis Block".to_string()))?;
    let block_number = storage.get_latest_block_number()?;
    let block_header = storage
        .get_block_header(block_number)?
        .ok_or(RLPxError::NotFound(format!("Block {block_number}")))?;

    let genesis = genesis_header.compute_block_hash();
    let block_hash = block_header.compute_block_hash();
    let fork_id = ForkId::new(
        chain_config,
        genesis_header,
        block_header.timestamp,
        block_number,
    );
    Ok(StatusMessage {
        eth_version,
        network_id,
        total_difficulty,
        block_hash,
        genesis,
        fork_id,
    })
}

pub fn validate_status(
    msg_data: StatusMessage,
    storage: &Store,
    eth_version: u32,
) -> Result<(), RLPxError> {
    let chain_config = storage.get_chain_config()?;

    // These blocks must always be available
    let genesis_header = storage
        .get_block_header(0)?
        .ok_or(RLPxError::NotFound("Genesis Block".to_string()))?;
    let genesis_hash = genesis_header.compute_block_hash();
    let latest_block_number = storage.get_latest_block_number()?;
    let latest_block_header = storage
        .get_block_header(latest_block_number)?
        .ok_or(RLPxError::NotFound(format!("Block {latest_block_number}")))?;
    let fork_id = ForkId::new(
        chain_config,
        genesis_header.clone(),
        latest_block_header.timestamp,
        latest_block_number,
    );

    //Check networkID
    if msg_data.network_id != chain_config.chain_id {
        return Err(RLPxError::HandshakeError(
            "Network Id does not match".to_string(),
        ));
    }
    //Check Protocol Version
    if msg_data.eth_version != eth_version {
        return Err(RLPxError::HandshakeError(
            "Eth protocol version does not match".to_string(),
        ));
    }
    //Check Genesis
    if msg_data.genesis != genesis_hash {
        return Err(RLPxError::HandshakeError(
            "Genesis does not match".to_string(),
        ));
    }
    // Check ForkID
    if !fork_id.is_valid(
        msg_data.fork_id,
        latest_block_number,
        latest_block_header.timestamp,
        chain_config,
        genesis_header,
    ) {
        return Err(RLPxError::HandshakeError("Invalid Fork Id".to_string()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_status;
    use crate::rlpx::eth::status::StatusMessage;
    use ethrex_common::{
        types::{ForkId, Genesis},
        H256, U256,
    };
    use ethrex_storage::{EngineType, Store};
    use std::{fs::File, io::BufReader};

    #[tokio::test]
    // TODO add tests for failing validations
    async fn test_validate_status() {
        // Setup
        // TODO we should have this setup exported to some test_utils module and use from there
        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        let file = File::open("../../../test_data/genesis-execution-api.json")
            .expect("Failed to open genesis file");
        let reader = BufReader::new(file);
        let genesis: Genesis =
            serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
        storage
            .add_initial_state(genesis.clone())
            .await
            .expect("Failed to add genesis block to DB");
        let config = genesis.config;
        let total_difficulty = U256::from(config.terminal_total_difficulty.unwrap_or_default());
        let genesis_header = genesis.get_block().header;
        let genesis_hash = genesis_header.compute_block_hash();
        let fork_id = ForkId::new(config, genesis_header, 2707305664, 123);

        let eth_version = 68;
        let message = StatusMessage {
            eth_version,
            network_id: 3503995874084926,
            total_difficulty,
            block_hash: H256::random(),
            genesis: genesis_hash,
            fork_id,
        };
        let result = validate_status(message, &storage, eth_version);
        assert!(result.is_ok());
    }
}
