use crate::rlpx::connection::server::send;
use crate::rlpx::l2::messages::{BatchSealed, L2Message, NewBlock};
use crate::rlpx::{connection::server::Established, error::PeerConnectionError, message::Message};
use ethereum_types::Address;
use ethereum_types::Signature;
use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::fork_choice::apply_fork_choice;
use ethrex_common::types::batch::Batch;
use ethrex_common::types::{Block, recover_address};
use ethrex_storage_rollup::StoreRollup;
use secp256k1::{Message as SecpMessage, SecretKey};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::time::Instant;
use tracing::{debug, error, info, warn};

use super::messages::batch_hash;
use super::{PERIODIC_BATCH_BROADCAST_INTERVAL, PERIODIC_BLOCK_BROADCAST_INTERVAL};

#[derive(Debug, Clone)]
pub struct L2ConnectedState {
    pub latest_block_sent: u64,
    pub latest_block_added: u64,
    pub latest_batch_sent: u64,
    pub blocks_on_queue: BTreeMap<u64, QueuedBlock>,
    pub batches_on_queue: BTreeMap<u64, Arc<Batch>>,
    pub store_rollup: StoreRollup,
    pub committer_key: Arc<SecretKey>,
    pub next_block_broadcast: Instant,
    pub next_batch_broadcast: Instant,
}

#[derive(Debug, Clone)]
pub struct QueuedBlock {
    pub block: Arc<Block>,
    pub fee_config: ethrex_common::types::fee_config::FeeConfig,
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

fn broadcast_message(state: &Established, msg: Message) -> Result<(), PeerConnectionError> {
    match msg {
        l2_msg @ Message::L2(_) => broadcast_l2_message(state, l2_msg),
        msg => {
            error!(
                peer=%state.node,
                message=%msg,
                "Broadcasting for this message is not supported"
            );
            let error_message = format!("Broadcasting for msg: {msg} is not supported");
            Err(PeerConnectionError::BroadcastError(error_message))
        }
    }
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

    pub(crate) fn connection_state_mut(
        &mut self,
    ) -> Result<&mut L2ConnectedState, PeerConnectionError> {
        match self {
            Self::Unsupported => Err(PeerConnectionError::IncompatibleProtocol),
            Self::Disconnected(_) => Err(PeerConnectionError::L2CapabilityNotNegotiated),
            Self::Connected(conn_state) => Ok(conn_state),
        }
    }
    pub(crate) fn connection_state(&self) -> Result<&L2ConnectedState, PeerConnectionError> {
        match self {
            Self::Unsupported => Err(PeerConnectionError::IncompatibleProtocol),
            Self::Disconnected(_) => Err(PeerConnectionError::L2CapabilityNotNegotiated),
            Self::Connected(conn_state) => Ok(conn_state),
        }
    }

    pub(crate) fn set_established(&mut self) -> Result<(), PeerConnectionError> {
        match self {
            Self::Unsupported => Err(PeerConnectionError::IncompatibleProtocol),
            Self::Disconnected(ctxt) => {
                let state = L2ConnectedState {
                    latest_block_sent: 0,
                    latest_block_added: 0,
                    blocks_on_queue: BTreeMap::new(),
                    batches_on_queue: BTreeMap::new(),
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
) -> Result<(), PeerConnectionError> {
    established.l2_state.connection_state()?;
    match msg {
        L2Message::BatchSealed(ref batch_sealed_msg) => {
            if should_process_batch_sealed(established, batch_sealed_msg).await? {
                established
                    .l2_state
                    .connection_state_mut()?
                    .batches_on_queue
                    .entry(batch_sealed_msg.batch.number)
                    .or_insert_with(|| batch_sealed_msg.batch.clone());
                broadcast_message(established, msg.into())?;
            }
            process_batches_on_queue(established).await?;
        }
        L2Message::NewBlock(ref new_block_msg) => {
            if should_process_new_block(established, new_block_msg).await? {
                established
                    .l2_state
                    .connection_state_mut()?
                    .blocks_on_queue
                    .entry(new_block_msg.block.header.number)
                    .or_insert_with(|| QueuedBlock {
                        block: new_block_msg.block.clone(),
                        fee_config: new_block_msg.fee_config,
                    });
                broadcast_message(established, msg.into())?;
            }
            process_blocks_on_queue(established).await?;
        }
    }
    Ok(())
}

pub(crate) async fn handle_l2_broadcast(
    state: &mut Established,
    l2_msg: &Message,
) -> Result<(), PeerConnectionError> {
    match l2_msg {
        msg @ Message::L2(L2Message::BatchSealed(_)) => send(state, msg.clone()).await,
        msg @ Message::L2(L2Message::NewBlock(_)) => send(state, msg.clone()).await,
        _ => Err(PeerConnectionError::BroadcastError(format!(
            "Message {:?} is not a valid L2 message for broadcast",
            l2_msg
        )))?,
    }
}

pub(crate) fn broadcast_l2_message(
    state: &Established,
    l2_msg: Message,
) -> Result<(), PeerConnectionError> {
    match l2_msg {
        msg @ Message::L2(L2Message::BatchSealed(_)) => {
            let task_id = tokio::task::id();
            state
                .connection_broadcast_send
                .send((task_id, msg.into()))
                .inspect_err(|e| {
                    error!(
                        peer=%state.node,
                        error=%e,
                        "Could not broadcast l2 message BatchSealed"
                    );
                })
                .map_err(|_| {
                    PeerConnectionError::BroadcastError(
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
                    error!(
                        peer=%state.node,
                        error=%e,
                        "Could not broadcast l2 message NewBlock",
                    );
                })
                .map_err(|_| {
                    PeerConnectionError::BroadcastError(
                        "Could not broadcast l2 message NewBlock".to_owned(),
                    )
                })?;
            Ok(())
        }
        _ => Err(PeerConnectionError::BroadcastError(format!(
            "Message {:?} is not a valid L2 message for broadcast",
            l2_msg
        ))),
    }
}
pub(crate) async fn send_new_block(
    established: &mut Established,
) -> Result<(), PeerConnectionError> {
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
                .ok_or(PeerConnectionError::InternalError(
                    "Block body not found after querying for the block number".to_owned(),
                ))?;
            let new_block_header = established.storage.get_block_header(block_number)?.ok_or(
                PeerConnectionError::InternalError(
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
                    let recovery_id: u8 =
                        Into::<i32>::into(recovery_id).try_into().map_err(|e| {
                            PeerConnectionError::InternalError(format!(
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

            let Some(fee_config) = l2_state
                .store_rollup
                .get_fee_config_by_block(block_number)
                .await?
            else {
                return Err(PeerConnectionError::InternalError(
                    "Fee config not found in rollup store for block".to_owned(),
                ));
            };

            NewBlock {
                block: new_block.into(),
                signature,
                fee_config,
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
) -> Result<bool, PeerConnectionError> {
    let l2_state = established.l2_state.connection_state_mut()?;
    if !established.blockchain.is_synced() {
        debug!("Not processing new block, blockchain is not synced");
        return Ok(false);
    }
    if established
        .storage
        .get_block_header(msg.block.header.number)?
        .is_some()
    {
        debug!(
            "Block {} received by peer already stored, ignoring it",
            msg.block.header.number
        );
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
            .map_err(|_| {
                PeerConnectionError::InternalError("Recover Address task failed".to_string())
            })?
            .map_err(|e| {
                error!(
                    peer=%established.node,
                    error=%e,
                    "Failed to recover lead sequencer",
                );
                PeerConnectionError::CryptographyError(e.to_string())
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
) -> Result<bool, PeerConnectionError> {
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
    let hash = batch_hash(&msg.batch);

    let recovered_lead_sequencer = recover_address(msg.signature, hash).map_err(|e| {
        error!(
            peer=%established.node,
            error=%e,
            "Failed to recover lead sequencer",
        );
        PeerConnectionError::CryptographyError(e.to_string())
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

pub async fn process_blocks_on_queue(
    established: &mut Established,
) -> Result<(), PeerConnectionError> {
    let l2_state = established.l2_state.connection_state_mut()?;

    let mut next_block_to_add = l2_state.latest_block_added + 1;
    if let Some(latest_batch_number) = l2_state.store_rollup.get_batch_number().await?
        && let Some(latest_batch) = l2_state
            .store_rollup
            .get_batch(latest_batch_number, ethrex_common::types::Fork::Prague)
            .await?
    {
        next_block_to_add = next_block_to_add.max(latest_batch.last_block + 1);
    }
    filter_potential_old_blocks(l2_state, next_block_to_add);

    while let Some(queued) = l2_state.blocks_on_queue.remove(&next_block_to_add) {
        let QueuedBlock { block, fee_config } = queued;
        // This check is necessary if a connection to another peer already applied the block but this connection
        // did not register that update.
        if let Ok(Some(_)) = established.storage.get_block_body(next_block_to_add).await {
            l2_state.latest_block_added = next_block_to_add;
            next_block_to_add += 1;
            continue;
        }
        let block_hash = block.hash();
        let block_number = block.header.number;
        let block = Arc::unwrap_or_clone(block);
        established
            .blockchain
            .add_block_pipeline(block)
            .inspect_err(|e| {
                error!(
                    peer=%established.node,
                    error=%e,
                    block_number,
                    ?block_hash,
                    "Error adding new block",
                );
            })?;

        apply_fork_choice(&established.storage, block_hash, block_hash, block_hash)
            .await
            .map_err(|e| {
                PeerConnectionError::BlockchainError(ChainError::Custom(format!(
                    "Error adding new block {} with hash {:?}, error: {e}",
                    block_number, block_hash
                )))
            })?;

        l2_state
            .store_rollup
            .store_fee_config_by_block(block_number, fee_config)
            .await?;
        info!(
            "Added new block {} with hash {:?}",
            next_block_to_add, block_hash
        );
        l2_state.latest_block_added = next_block_to_add;
        next_block_to_add += 1;
    }
    Ok(())
}

fn filter_potential_old_blocks(l2_state: &mut L2ConnectedState, next_block_to_add: u64) {
    let keys_to_remove = if let Some(block_entry) = l2_state.blocks_on_queue.first_entry()
        && block_entry.key() < &next_block_to_add
    {
        let mut keys = vec![];
        for key in l2_state.blocks_on_queue.keys() {
            if *key < next_block_to_add {
                keys.push(*key);
            } else {
                break;
            }
        }
        keys
    } else {
        vec![]
    };
    for key in keys_to_remove {
        l2_state.blocks_on_queue.remove(&key);
    }
}

pub(crate) async fn send_sealed_batch(
    established: &mut Established,
) -> Result<(), PeerConnectionError> {
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
        let l1_fork = established.blockchain.current_fork().await?;
        let Some(batch) = l2_state
            .store_rollup
            .get_batch(next_batch_to_send, l1_fork)
            .await?
        else {
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

pub async fn process_batches_on_queue(
    established: &mut Established,
) -> Result<(), PeerConnectionError> {
    let l2_state = established.l2_state.connection_state_mut()?;
    let Some(latest_stored_batch) = l2_state.store_rollup.get_batch_number().await? else {
        return Ok(());
    };
    let mut next_batch_to_seal = latest_stored_batch + 1;
    while let Some(batch) = l2_state.batches_on_queue.get(&next_batch_to_seal) {
        let last_block_on_next_batch = batch.last_block;
        if established
            .storage
            .get_block_by_number(last_block_on_next_batch)
            .await?
            .is_none()
        {
            debug!("Missing blocks from the next batch to seal");
            return Ok(());
        }
        let Some(batch) = l2_state.batches_on_queue.remove(&next_batch_to_seal) else {
            return Ok(());
        };
        let batch = Arc::unwrap_or_clone(batch);
        let (batch_number, batch_first_block, batch_last_block) =
            (batch.number, batch.first_block, batch.last_block);
        l2_state.store_rollup.seal_batch(batch).await?;
        info!(
            "Sealed batch {} with blocks from {} to {}",
            batch_number, batch_first_block, batch_last_block
        );

        next_batch_to_seal += 1;
    }

    Ok(())
}

// These tests are disabled because they previously assumed
// the connection used the old struct RLPxConnection, but
// the new GenServer approach changes a lot of things,
// this will be eventually addressed (#3563)
#[cfg(test)]
mod tests {}
