use ethrex_common::types::ForkId;
use ethrex_storage::{Store, error::StoreError};

use crate::rlpx::{error::PeerConnectionError, eth::status::StatusMessage, p2p::Capability};

pub async fn validate_status<ST: StatusMessage>(
    msg_data: ST,
    storage: &Store,
    eth_capability: &Capability,
) -> Result<(), PeerConnectionError> {
    //Check networkID
    let chain_config = storage.get_chain_config();
    if msg_data.get_network_id() != chain_config.chain_id {
        return Err(PeerConnectionError::HandshakeError(
            "Network Id does not match".to_string(),
        ));
    }
    //Check Protocol Version
    if msg_data.get_eth_version() != eth_capability.version {
        return Err(PeerConnectionError::HandshakeError(
            "Eth protocol version does not match".to_string(),
        ));
    }
    //Check Genesis
    let genesis_header = storage
        .get_block_header(0)?
        .ok_or(PeerConnectionError::NotFound("Genesis Block".to_string()))?;
    let genesis_hash = genesis_header.hash();
    if msg_data.get_genesis() != genesis_hash {
        return Err(PeerConnectionError::HandshakeError(
            "Genesis does not match".to_string(),
        ));
    }
    // Check ForkID
    if !is_fork_id_valid(storage, &msg_data.get_fork_id()).await? {
        return Err(PeerConnectionError::HandshakeError(
            "Invalid Fork Id".to_string(),
        ));
    }
    Ok(())
}

/// Validates the fork id from a remote node is valid.
pub async fn is_fork_id_valid(
    storage: &Store,
    remote_fork_id: &ForkId,
) -> Result<bool, StoreError> {
    let chain_config = storage.get_chain_config();
    let genesis_header = storage
        .get_block_header(0)?
        .ok_or(StoreError::Custom("Latest block not in DB".to_string()))?;
    let latest_block_number = storage.get_latest_block_number().await?;
    let latest_block_header = storage
        .get_block_header(latest_block_number)?
        .ok_or(StoreError::Custom("Latest block not in DB".to_string()))?;
    let local_fork_id = ForkId::new(
        chain_config,
        genesis_header.clone(),
        latest_block_header.timestamp,
        latest_block_number,
    );
    Ok(local_fork_id.is_valid(
        remote_fork_id.clone(),
        latest_block_number,
        latest_block_header.timestamp,
        chain_config,
        genesis_header,
    ))
}

#[cfg(test)]
mod tests {
    use super::validate_status;
    use crate::rlpx::eth::eth68::status::StatusMessage68;
    use crate::rlpx::p2p::Capability;
    use ethrex_common::{
        H256, U256,
        types::{ForkId, Genesis},
    };

    use ethrex_storage::{EngineType, Store};
    use std::{fs::File, io::BufReader};

    #[tokio::test]
    // TODO add tests for failing validations
    async fn test_validate_status() {
        // Setup
        // TODO we should have this setup exported to some test_utils module and use from there
        let mut storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        let file = File::open("../../../fixtures/genesis/execution-api.json")
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
        let genesis_hash = genesis_header.hash();
        let fork_id = ForkId::new(config, genesis_header, 2707305664, 123);

        let eth = Capability::eth(68);
        let message = StatusMessage68 {
            eth_version: eth.version,
            network_id: 3503995874084926,
            total_difficulty,
            block_hash: H256::random(),
            genesis: genesis_hash,
            fork_id,
        };
        let result = validate_status(message, &storage, &eth).await;
        assert!(result.is_ok());
    }
}
