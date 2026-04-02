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
