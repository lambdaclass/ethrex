use crate::rlpx::connection::server::{broadcast_message, send};
use crate::rlpx::l2::messages::{BatchSealed, L2Message, NewBlock};
use crate::rlpx::utils::log_peer_error;
use crate::rlpx::{connection::server::Established, error::RLPxError, message::Message};
use ethereum_types::Address;
use ethereum_types::Signature;
use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::fork_choice::apply_fork_choice;
use ethrex_common::types::{Block, recover_address};
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
    pub latest_batch_sent: u64,
    pub blocks_on_queue: BTreeMap<u64, Arc<Block>>,
    pub store_rollup: StoreRollup,
    pub committer_key: Arc<SecretKey>,
    pub next_block_broadcast: Instant,
    pub next_batch_broadcast: Instant,
}

#[derive(Debug, Clone)]
pub struct P2PBasedContext {
    pub store_rollup: StoreRollup,
    pub committer_key: Arc<SecretKey>,
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
    pub(crate) fn is_supported(&self) -> bool {
        match self {
            Self::Unsupported => false,
            Self::Disconnected(_) | Self::Connected(_) => true,
        }
    }

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

    pub(crate) fn set_established(&mut self) -> Result<(), RLPxError> {
        match self {
            Self::Unsupported => Err(RLPxError::IncompatibleProtocol),
            Self::Disconnected(ctxt) => {
                let state = L2ConnectedState {
                    latest_block_sent: 0,
                    latest_block_added: 0,
                    blocks_on_queue: BTreeMap::new(),
                    latest_batch_sent: 0,
                    store_rollup: ctxt.store_rollup.clone(),
                    committer_key: ctxt.committer_key.clone(),
                    next_block_broadcast: Instant::now() + PERIODIC_BLOCK_BROADCAST_INTERVAL,
                    next_batch_broadcast: Instant::now() + PERIODIC_BATCH_BROADCAST_INTERVAL,
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
                broadcast_message(established, msg.into())?;
            }
        }
        L2Message::NewBlock(ref new_block_msg) => {
            if should_process_new_block(established, new_block_msg).await? {
                process_new_block(established, new_block_msg).await?;
                broadcast_message(established, msg.into())?;
            }
        }
    }
    Ok(())
}

pub(crate) async fn handle_l2_broadcast(
    state: &mut Established,
    l2_msg: &Message,
) -> Result<(), RLPxError> {
    match l2_msg {
        msg @ Message::L2(L2Message::BatchSealed(_)) => send(state, msg.clone()).await,
        msg @ Message::L2(L2Message::NewBlock(_)) => send(state, msg.clone()).await,
        _ => Err(RLPxError::BroadcastError(format!(
            "Message {:?} is not a valid L2 message for broadcast",
            l2_msg
        )))?,
    }
}

pub(crate) fn broadcast_l2_message(state: &Established, l2_msg: Message) -> Result<(), RLPxError> {
    match l2_msg {
        msg @ Message::L2(L2Message::BatchSealed(_)) => {
            let task_id = tokio::task::id();
            state
                .connection_broadcast_send
                .send((task_id, msg.into()))
                .inspect_err(|e| {
                    log_peer_error(
                        &state.node,
                        &format!("Could not broadcast l2 message BatchSealed: {e}"),
                    );
                })
                .map_err(|_| {
                    RLPxError::BroadcastError(
                        "Could not broadcast l2 message BatchSealed".to_owned(),
                    )
                })?;
            Ok(())
        }
        msg @ Message::L2(L2Message::NewBlock(_)) => {
            let task_id = tokio::task::id();
            state
                .connection_broadcast_send
                .send((task_id, msg.into()))
                .inspect_err(|e| {
                    log_peer_error(
                        &state.node,
                        &format!("Could not broadcast l2 message NewBlock: {e}"),
                    );
                })
                .map_err(|_| {
                    RLPxError::BroadcastError("Could not broadcast l2 message NewBlock".to_owned())
                })?;
            Ok(())
        }
        _ => Err(RLPxError::BroadcastError(format!(
            "Message {:?} is not a valid L2 message for broadcast",
            l2_msg
        ))),
    }
}
pub(crate) async fn send_new_block(established: &mut Established) -> Result<(), RLPxError> {
    let latest_block_number = established.storage.get_latest_block_number().await?;
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
            let signature = match l2_state
                .store_rollup
                .get_signature_by_block(new_block.hash())
                .await?
            {
                Some(sig) => sig,
                None => {
                    let (recovery_id, signature) = secp256k1::SECP256K1
                        .sign_ecdsa_recoverable(
                            &SecpMessage::from_digest(new_block.hash().to_fixed_bytes()),
                            &l2_state.committer_key,
                        )
                        .serialize_compact();
                    let recovery_id: u8 = Into::<i32>::into(recovery_id).try_into().map_err(|e| {
                        RLPxError::InternalError(format!(
                            "Failed to convert recovery id to u8: {e}. This is a bug."
                        ))
                    })?;
                    let mut sig = [0u8; 65];
                    sig[..64].copy_from_slice(&signature);
                    sig[64] = recovery_id;
                    let signature = Signature::from_slice(&sig);
                    l2_state
                        .store_rollup
                        .store_signature_by_block(new_block.hash(), signature)
                        .await?;
                    signature
                }
            };
            NewBlock {
                block: new_block.into(),
                signature,
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
    if l2_state.latest_block_added >= msg.block.header.number
        || l2_state
            .blocks_on_queue
            .contains_key(&msg.block.header.number)
    {
        debug!(
            "Block {} received by peer already stored, ignoring it",
            msg.block.header.number
        );
        return Ok(false);
    }

    let block_hash = msg.block.hash();

    let msg_signature = msg.signature;
    let recovered_lead_sequencer =
        tokio::task::spawn_blocking(move || recover_address(msg_signature, block_hash))
            .await
            .map_err(|_| RLPxError::InternalError("Recover Address task failed".to_string()))?
            .map_err(|e| {
                log_peer_error(
                    &established.node,
                    &format!("Failed to recover lead sequencer: {e}"),
                );
                RLPxError::CryptographyError(e.to_string())
            })?;

    if !validate_signature(recovered_lead_sequencer) {
        return Ok(false);
    }
    l2_state
        .store_rollup
        .store_signature_by_block(block_hash, msg.signature)
        .await?;
    Ok(true)
}

async fn should_process_batch_sealed(
    established: &mut Established,
    msg: &BatchSealed,
) -> Result<bool, RLPxError> {
    let l2_state = established.l2_state.connection_state_mut()?;
    if !established.blockchain.is_synced() {
        debug!("Not processing BatchSealedMessage, blockchain is not synced");
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
        debug!(
            "Not processing batch {} because the last block {} is not added yet",
            msg.batch.number, msg.batch.last_block
        );
        return Ok(false);
    }

    let hash = batch_hash(&msg.batch);

    let recovered_lead_sequencer = recover_address(msg.signature, hash).map_err(|e| {
        log_peer_error(
            &established.node,
            &format!("Failed to recover lead sequencer: {e}"),
        );
        RLPxError::CryptographyError(e.to_string())
    })?;

    if !validate_signature(recovered_lead_sequencer) {
        return Ok(false);
    }
    l2_state
        .store_rollup
        .store_signature_by_batch(msg.batch.number, msg.signature)
        .await?;
    Ok(true)
}

async fn process_new_block(established: &mut Established, msg: &NewBlock) -> Result<(), RLPxError> {
    let l2_state = established.l2_state.connection_state_mut()?;
    l2_state
        .blocks_on_queue
        .entry(msg.block.header.number)
        .or_insert_with(|| msg.block.clone());

    let mut next_block_to_add = l2_state.latest_block_added + 1;
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
             defaulting to signing with committer key: {err}"
                )
            }) {
            Ok(Some(recovered_sig)) => BatchSealed::new(batch, recovered_sig),
            Ok(None) | Err(_) => {
                let msg = BatchSealed::from_batch_and_key(
                    batch,
                    l2_state.committer_key.clone().as_ref(),
                )?;
                l2_state
                    .store_rollup
                    .store_signature_by_batch(msg.batch.number, msg.signature)
                    .await?;
                msg
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
