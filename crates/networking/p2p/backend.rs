use ethrex_common::types::ForkId;
use ethrex_polygon::{
    fork_id::{polygon_fork_id, polygon_is_fork_id_valid},
    genesis::bor_config_for_chain,
};
use ethrex_storage::{Store, error::StoreError};
use tracing::debug;

use crate::rlpx::{error::PeerConnectionError, eth::status::StatusMessage, p2p::Capability};

use ethrex_common::types::BlockHash;

/// Validates a remote status message and returns the remote peer's head block hash.
pub async fn validate_status<ST: StatusMessage>(
    msg_data: ST,
    storage: &Store,
    eth_capability: &Capability,
) -> Result<BlockHash, PeerConnectionError> {
    //Check networkID
    let chain_config = storage.get_chain_config();
    if msg_data.get_network_id() != chain_config.chain_id {
        return Err(PeerConnectionError::HandshakeError(
            "Network Id does not match".to_string(),
        ));
    }
    //Check Protocol Version
    // Allow eth/68 status when eth/69 was negotiated: some clients (e.g. Bor)
    // advertise eth/69 but still send the legacy eth/68-shaped status message.
    let version_ok = msg_data.get_eth_version() == eth_capability.version
        || (eth_capability.version == 69 && msg_data.get_eth_version() == 68);
    if !version_ok {
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
    let remote_fork_id = msg_data.get_fork_id();
    if !is_fork_id_valid(storage, &remote_fork_id).await? {
        let local_fork_id = get_fork_id(storage).await.ok();
        let is_polygon = ethrex_polygon::genesis::is_polygon_chain(chain_config.chain_id);
        if is_polygon {
            // Polygon: old Bor versions compute fork IDs with only EVM forks
            // (no Bor-specific forks). Warn but allow the connection — chain ID
            // and genesis hash already guarantee we're on the right network.
            debug!(
                ?remote_fork_id,
                ?local_fork_id,
                "Polygon fork ID mismatch (old Bor node?) — allowing connection"
            );
        } else {
            debug!(?remote_fork_id, ?local_fork_id, "Fork ID validation failed");
            return Err(PeerConnectionError::HandshakeError(
                "Invalid Fork Id".to_string(),
            ));
        }
    }
    Ok(msg_data.get_block_hash())
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

    // Polygon uses its own fork schedule — validate against Polygon forks.
    if let Some(bor_config) = bor_config_for_chain(chain_config.chain_id) {
        let genesis_hash = genesis_header.hash();
        return Ok(polygon_is_fork_id_valid(
            genesis_hash,
            bor_config,
            latest_block_number,
            remote_fork_id,
        ));
    }

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

/// Returns the correct fork ID for the current network.
///
/// Uses `polygon_fork_id` for Polygon networks (chain 137, 80002),
/// falls back to the standard Ethereum `ForkId::new` otherwise.
/// Call sites in P2P discovery should use this instead of `storage.get_fork_id()`.
pub async fn get_fork_id(storage: &Store) -> Result<ForkId, StoreError> {
    let chain_config = storage.get_chain_config();
    if let Some(bor_config) = bor_config_for_chain(chain_config.chain_id) {
        let genesis_header = storage
            .get_block_header(0)?
            .ok_or(StoreError::Custom("Genesis block not in DB".to_string()))?;
        let latest_block_number = storage.get_latest_block_number().await?;
        return Ok(polygon_fork_id(
            genesis_header.hash(),
            bor_config,
            latest_block_number,
        ));
    }
    storage.get_fork_id().await
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
