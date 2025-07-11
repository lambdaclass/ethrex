use crate::rlpx::connection::server::{broadcast_message, send};
use crate::rlpx::l2::messages::{BatchSealed, GetBatchSealedResponse, L2Message, NewBlock};
use crate::rlpx::utils::{get_pub_key, log_peer_error};
use crate::rlpx::{connection::server::Established, error::RLPxError, message::Message};
use ethereum_types::Address;
use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::fork_choice::apply_fork_choice;
use ethrex_blockchain::sequencer_state::{SequencerState, SequencerStatus};
use ethrex_common::types::Block;
use ethrex_storage_rollup::StoreRollup;
use secp256k1::{Message as SecpMessage, SecretKey};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::time::Instant;
use tracing::{debug, info, warn};

use super::messages::batch_hash;
use super::{PERIODIC_BATCH_BROADCAST_INTERVAL, PERIODIC_BLOCK_BROADCAST_INTERVAL};

#[derive(Debug, Clone)]
pub struct L2ConnectedState {
    pub latest_block_sent: u64,
    pub latest_block_added: u64,
    pub last_block_broadcasted: u64,
    pub latest_batch_sent: u64,
    pub blocks_on_queue: BTreeMap<u64, Arc<Block>>,
    pub store_rollup: StoreRollup,
    pub committer_key: Arc<SecretKey>,
    pub next_block_broadcast: Instant,
    pub next_batch_broadcast: Instant,
    pub sequencer_state: SequencerState,
}

#[derive(Debug, Clone)]
pub struct P2PBasedContext {
    pub store_rollup: StoreRollup,
    pub committer_key: Arc<SecretKey>,
    pub sequencer_state: SequencerState,
}

#[derive(Debug, Clone)]
pub enum L2ConnState {
    Unsupported,
    Disconnected(P2PBasedContext),
    Connected(L2ConnectedState),
}

#[derive(Debug, Clone)]
pub enum L2Cast {
    BlockBroadcast,
    BatchBroadcast,
}

impl L2ConnState {
    pub(crate) fn connection_state_mut(&mut self) -> Result<&mut L2ConnectedState, RLPxError> {
        match self {
            Self::Unsupported => Err(RLPxError::IncompatibleProtocol),
            Self::Disconnected(_) => Err(RLPxError::L2CapabilityNotNegotiated),
            Self::Connected(conn_state) => Ok(conn_state),
        }
    }
    pub(crate) fn connection_state(&self) -> Result<&L2ConnectedState, RLPxError> {
        match self {
            Self::Unsupported => Err(RLPxError::IncompatibleProtocol),
            Self::Disconnected(_) => Err(RLPxError::L2CapabilityNotNegotiated),
            Self::Connected(conn_state) => Ok(conn_state),
        }
    }

    pub(crate) async fn set_established(&mut self) -> Result<(), RLPxError> {
        match self {
            Self::Unsupported => Err(RLPxError::IncompatibleProtocol),
            Self::Disconnected(ctxt) => {
                let latest_batch_on_store = ctxt
                    .store_rollup
                    .get_latest_batch_number()
                    .await
                    .unwrap_or(0);
                // Get the latest block number from the latest batch, or default to 0
                let latest_block_on_store = match ctxt
                    .store_rollup
                    .get_block_numbers_by_batch(latest_batch_on_store)
                    .await?
                {
                    Some(block_numbers) if !block_numbers.is_empty() => {
                        *block_numbers.last().expect("Batch cannot be empty")
                    }
                    _ => 0,
                };
                let state = L2ConnectedState {
                    latest_block_sent: latest_block_on_store,
                    latest_block_added: latest_block_on_store,
                    last_block_broadcasted: latest_block_on_store,
                    blocks_on_queue: BTreeMap::new(),
                    latest_batch_sent: latest_batch_on_store,
                    store_rollup: ctxt.store_rollup.clone(),
                    committer_key: ctxt.committer_key.clone(),
                    next_block_broadcast: Instant::now() + PERIODIC_BLOCK_BROADCAST_INTERVAL,
                    next_batch_broadcast: Instant::now() + PERIODIC_BATCH_BROADCAST_INTERVAL,
                    sequencer_state: ctxt.sequencer_state.clone(),
                };
                *self = L2ConnState::Connected(state);
                Ok(())
            }
            Self::Connected(_) => Ok(()),
        }
    }
}

fn validate_signature(_recovered_lead_sequencer: Address) -> bool {
    // Until the RPC module can be included in the P2P crate, we skip the validation
    true
}

pub(crate) async fn handle_based_capability_message(
    established: &mut Established,
    msg: L2Message,
) -> Result<(), RLPxError> {
    established.l2_state.connection_state()?;
    match msg {
        L2Message::BatchSealed(ref batch_sealed_msg) => {
            if should_process_batch_sealed(established, batch_sealed_msg).await? {
                process_batch_sealed(established, batch_sealed_msg).await?;
                let new_latest_batch_sent = batch_sealed_msg.batch.number;
                broadcast_message(established, msg.into())?;
                established
                    .l2_state
                    .connection_state_mut()?
                    .latest_batch_sent = new_latest_batch_sent;
            }
        }
        L2Message::NewBlock(ref new_block_msg) => {
            if let SequencerStatus::Following = established
                .l2_state
                .connection_state()?
                .sequencer_state
                .status()
                .await
            {
                if should_process_new_block(established, new_block_msg).await? {
                    debug!(
                        "adding block to queue: {}",
                        new_block_msg.block.header.number
                    );
                    established
                        .l2_state
                        .connection_state_mut()?
                        .blocks_on_queue
                        .entry(new_block_msg.block.header.number)
                        .or_insert_with(|| new_block_msg.block.clone());
                    if new_block_msg.block.header.number
                        > established
                            .l2_state
                            .connection_state()?
                            .last_block_broadcasted
                    {
                        broadcast_message(
                            established,
                            Message::L2(L2Message::NewBlock(new_block_msg.clone())),
                        )?;
                        established
                            .l2_state
                            .connection_state_mut()?
                            .last_block_broadcasted = new_block_msg.block.header.number;
                    }
                }
                // If the sequencer is following, we process the new block
                // to keep the state updated.
                process_blocks_on_queue(established).await?;
            }
        }
        L2Message::GetBatchSealed(_req) => {
            let l2_state = established.l2_state.connection_state()?;
            if let SequencerStatus::Syncing = l2_state.sequencer_state.status().await {
                // If the sequencer is syncing, we won't send any batches
                let response = GetBatchSealedResponse {
                    batches: vec![], // empty response
                                     // for now skipping signatures
                };
                send(established, response.into()).await?;
                return Ok(());
            }
            let mut batches = vec![];
            for batch_number in _req.first_batch..=_req.last_batch {
                let Some(batch) = l2_state.store_rollup.get_batch(batch_number).await? else {
                    return Err(RLPxError::InternalError(
                        "The node is not syncing and was asked for a batch it does not have"
                            .to_string(),
                    ));
                };
                batches.push(batch);
            }

            let response = GetBatchSealedResponse {
                batches,
                // for now skipping signatures
            };
            send(established, response.into()).await?;
        }
        L2Message::GetBatchSealedResponse(_req) => {
            return Err(RLPxError::InternalError("Unexpected GetBatchSealedResponse message received, this message should had been sent to the backend".to_string()));
        }
    }
    Ok(())
}

pub(crate) async fn send_new_block(established: &mut Established) -> Result<(), RLPxError> {
    let latest_block_number = established.storage.get_latest_block_number().await?;
    if !established.blockchain.is_synced() {
        // We do this since we are syncing, we don't want to broadcast old blocks
        established
            .l2_state
            .connection_state_mut()?
            .latest_block_sent = latest_block_number;
        return Ok(());
    }
    let latest_block_sent = established
        .l2_state
        .connection_state_mut()?
        .latest_block_sent;
    for block_number in latest_block_sent + 1..=latest_block_number {
        let new_block_msg = {
            let l2_state = established.l2_state.connection_state_mut()?;
            debug!(
                "Broadcasting new block, current: {}, last broadcasted: {}",
                block_number, l2_state.latest_block_sent
            );

            let new_block_body = established
                .storage
                .get_block_body(block_number)
                .await?
                .ok_or(RLPxError::InternalError(
                    "Block body not found after querying for the block number".to_owned(),
                ))?;
            let new_block_header = established.storage.get_block_header(block_number)?.ok_or(
                RLPxError::InternalError(
                    "Block header not found after querying for the block number".to_owned(),
                ),
            )?;
            let new_block = Block {
                header: new_block_header,
                body: new_block_body,
            };
            let (signature, recovery_id) = if let Some(recovered_sig) = l2_state
                .store_rollup
                .get_signature_by_block(new_block.hash())
                .await?
            {
                let mut signature = [0u8; 64];
                let mut recovery_id = [0u8; 4];
                signature.copy_from_slice(&recovered_sig[..64]);
                recovery_id.copy_from_slice(&recovered_sig[64..68]);
                (signature, recovery_id)
            } else {
                let (recovery_id, signature) = secp256k1::SECP256K1
                    .sign_ecdsa_recoverable(
                        &SecpMessage::from_digest(new_block.hash().to_fixed_bytes()),
                        &l2_state.committer_key,
                    )
                    .serialize_compact();
                let recovery_id: [u8; 4] = recovery_id.to_i32().to_be_bytes();
                (signature, recovery_id)
            };
            NewBlock {
                block: new_block.into(),
                signature,
                recovery_id,
            }
        };

        send(established, new_block_msg.into()).await?;
        established
            .l2_state
            .connection_state_mut()?
            .latest_block_sent = block_number;
    }

    Ok(())
}

async fn should_process_new_block(
    established: &mut Established,
    msg: &NewBlock,
) -> Result<bool, RLPxError> {
    let l2_state = established.l2_state.connection_state_mut()?;
    if !established.blockchain.is_synced() {
        debug!("Not processing new block, blockchain is not synced");
        return Ok(false);
    }
    if l2_state
        .blocks_on_queue
        .contains_key(&msg.block.header.number)
    {
        debug!(
            "Block {} received already on queue, ignoring it",
            msg.block.header.number
        );
        return Ok(false);
    }

    let block_hash = msg.block.hash();

    let recovered_lead_sequencer = get_pub_key(
        msg.recovery_id,
        &msg.signature,
        *block_hash.as_fixed_bytes(),
    )
    .map_err(|e| {
        log_peer_error(
            &established.node,
            &format!("Failed to recover lead sequencer: {e}"),
        );
        RLPxError::CryptographyError(e.to_string())
    })?;

    if !validate_signature(recovered_lead_sequencer) {
        l2_state.blocks_on_queue.remove(&msg.block.header.number);
        return Ok(false);
    }
    let mut signature = [0u8; 68];
    signature[..64].copy_from_slice(&msg.signature[..]);
    signature[64..].copy_from_slice(&msg.recovery_id[..]);
    l2_state
        .store_rollup
        .store_signature_by_block(block_hash, signature)
        .await?;
    Ok(true)
}

async fn should_process_batch_sealed(
    established: &mut Established,
    msg: &BatchSealed,
) -> Result<bool, RLPxError> {
    let l2_state = established.l2_state.connection_state_mut()?;
    if !established.blockchain.is_synced() {
        debug!("Not processing new block, blockchain is not synced");
        return Ok(false);
    }
    if l2_state
        .store_rollup
        .contains_batch(&msg.batch.number)
        .await?
    {
        debug!("Batch {} already sealed, ignoring it", msg.batch.number);
        return Ok(false);
    }
    if msg.batch.first_block == msg.batch.last_block {
        // is empty batch
        return Ok(false);
    }
    if l2_state.latest_block_added < msg.batch.last_block {
        let latest_block_in_storage = established.storage.get_latest_block_number().await?;
        l2_state.latest_block_added = latest_block_in_storage;

        if l2_state.latest_block_added < msg.batch.last_block {
            debug!(
                "Not processing batch {} because the last block {} is not added yet",
                msg.batch.number, msg.batch.last_block
            );
            return Ok(false);
        }
    }

    let hash = batch_hash(&msg.batch);

    let recovered_lead_sequencer =
        get_pub_key(msg.recovery_id, &msg.signature, hash.0).map_err(|e| {
            log_peer_error(
                &established.node,
                &format!("Failed to recover lead sequencer: {e}"),
            );
            RLPxError::CryptographyError(e.to_string())
        })?;

    if !validate_signature(recovered_lead_sequencer) {
        return Ok(false);
    }

    let mut signature = [0u8; 68];
    signature[..64].copy_from_slice(&msg.signature[..]);
    signature[64..].copy_from_slice(&msg.recovery_id[..]);
    l2_state
        .store_rollup
        .store_signature_by_batch(msg.batch.number, signature)
        .await?;
    Ok(true)
}

async fn process_blocks_on_queue(established: &mut Established) -> Result<(), RLPxError> {
    let l2_state = established.l2_state.connection_state_mut()?;
    if !established.blockchain.is_synced() {
        let latest_block_in_storage = established.storage.get_latest_block_number().await?;
        l2_state.latest_block_added = latest_block_in_storage;
        return Ok(());
    }

    let mut next_block_to_add = l2_state.latest_block_added + 1;
    let latest_batch_number = l2_state.store_rollup.get_latest_batch_number().await?;
    if let Some(latest_batch) = l2_state.store_rollup.get_batch(latest_batch_number).await? {
        next_block_to_add = next_block_to_add.max(latest_batch.last_block + 1);
    }
    while let Some(block) = l2_state.blocks_on_queue.remove(&next_block_to_add) {
        // This check is necessary if a connection to another peer already applied the block but this connection
        // did not register that update.
        if let Ok(Some(_)) = established.storage.get_block_body(next_block_to_add).await {
            l2_state.latest_block_added = next_block_to_add;
            next_block_to_add += 1;
            continue;
        }
        established
            .blockchain
            .add_block(&block)
            .await
            .inspect_err(|e| {
                log_peer_error(
                    &established.node,
                    &format!(
                        "Error adding new block {} with hash {:?}, error: {e}",
                        block.header.number,
                        block.hash()
                    ),
                );
            })?;
        let block_hash = block.hash();

        apply_fork_choice(&established.storage, block_hash, block_hash, block_hash)
            .await
            .map_err(|e| {
                RLPxError::BlockchainError(ChainError::Custom(format!(
                    "Error adding new block {} with hash {:?}, error: {e}",
                    block.header.number,
                    block.hash()
                )))
            })?;
        info!(
            "Added new block {} with hash {:?}",
            next_block_to_add, block_hash
        );
        l2_state.latest_block_added = next_block_to_add;
        next_block_to_add += 1;
    }
    Ok(())
}

pub(crate) async fn send_sealed_batch(established: &mut Established) -> Result<(), RLPxError> {
    if !established.blockchain.is_synced() {
        // We do this since we are syncing, we don't want to broadcast old batches
        let latest_batch_number = established
            .l2_state
            .connection_state()?
            .store_rollup
            .get_latest_batch_number()
            .await?;
        established
            .l2_state
            .connection_state_mut()?
            .latest_batch_sent = latest_batch_number;
        return Ok(());
    }
    let batch_sealed_msg = {
        let l2_state = established.l2_state.connection_state_mut()?;
        let next_batch_to_send = l2_state.latest_batch_sent + 1;
        if !l2_state
            .store_rollup
            .contains_batch(&next_batch_to_send)
            .await?
        {
            return Ok(());
        }
        let Some(batch) = l2_state.store_rollup.get_batch(next_batch_to_send).await? else {
            return Ok(());
        };
        match l2_state
            .store_rollup
            .get_signature_by_batch(next_batch_to_send)
            .await
            .inspect_err(|err| {
                warn!(
                    "Fetching signature from store returned an error, \
             defaulting to signing with commiter key: {err}"
                )
            }) {
            Ok(Some(recovered_sig)) => {
                let (signature, recovery_id) = {
                    let mut signature = [0u8; 64];
                    let mut recovery_id = [0u8; 4];
                    signature.copy_from_slice(&recovered_sig[..64]);
                    recovery_id.copy_from_slice(&recovered_sig[64..68]);
                    (signature, recovery_id)
                };
                BatchSealed::new(batch, signature, recovery_id)
            }
            Ok(None) | Err(_) => {
                BatchSealed::from_batch_and_key(batch, l2_state.committer_key.clone().as_ref())
            }
        }
    };
    let batch_sealed_msg: Message = batch_sealed_msg.into();
    send(established, batch_sealed_msg).await?;
    established
        .l2_state
        .connection_state_mut()?
        .latest_batch_sent += 1;
    Ok(())
}

async fn process_batch_sealed(
    established: &mut Established,
    msg: &BatchSealed,
) -> Result<(), RLPxError> {
    let l2_state = established.l2_state.connection_state_mut()?;
    l2_state.store_rollup.seal_batch(*msg.batch.clone()).await?;
    info!(
        "Sealed batch {} with blocks from {} to {}",
        msg.batch.number, msg.batch.first_block, msg.batch.last_block
    );
    Ok(())
}

// These tests are disabled because they previously assumed
// the connection used the old struct RLPxConnection, but
// the new GenServer approach changes a lot of things,
// this will be eventually addressed (#3563)
#[cfg(test)]
mod tests {}
