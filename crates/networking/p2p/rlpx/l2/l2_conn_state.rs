use std::collections::BTreeMap;

use crate::rlpx::based::get_hash_batch_sealed;
use crate::rlpx::utils::{get_pub_key, log_peer_error};
use crate::rlpx::{connection::RLPxConnection, error::RLPxError, message::Message};
use crate::rlpx::l2::messages::{NewBlockMessage, BatchSealedMessage};
use ethrex_common::types::Block;
use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};
use ethereum_types::Address;
use secp256k1::{Message as SecpMessage, SecretKey};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{debug, info};
#[derive(Debug, Clone)]
pub struct L2ConnState {
    pub latest_block_sent: u64,
    pub latest_block_added: u64,
    pub latest_batch_sent: u64,
    pub blocks_on_queue: BTreeMap<u64, Block>,
    pub store_rollup: StoreRollup,
    pub commiter_key: Option<SecretKey>,
}

pub async fn send_new_block<S: AsyncWrite + AsyncRead + std::marker::Unpin>(conn: &mut RLPxConnection<S>) -> Result<(), RLPxError> {
    // This section is conditionally compiled based on the "l2" feature flag due to dependencies on the rollup store.
    let latest_block_number = conn.storage.get_latest_block_number().await?;
    let latest_block_sent = conn.l2_state.clone().ok_or(RLPxError::IncompatibleProtocol)?.latest_block_sent;
    // FIXME:
    // 1. Check if I can avoid the partial borrow below
    // 2. Check if we can make this a method for L2 State instead.
    for i in latest_block_sent + 1..=latest_block_number {
        debug!(
            "Broadcasting new block, current: {}, last broadcasted: {}",
            i, latest_block_sent
        );
        let (signature, recovery_id, new_block) =
        {
            let Some(ref mut conn_l2_state) = conn.l2_state else {
                return Err(RLPxError::IncompatibleProtocol)
            };

            let new_block_body =
                conn.storage
                    .get_block_body(i)
                    .await?
                    .ok_or(RLPxError::InternalError(
                        "Block body not found after querying for the block number".to_owned(),
                    ))?;
            let new_block_header =
                conn.storage
                    .get_block_header(i)?
                    .ok_or(RLPxError::InternalError(
                        "Block header not found after querying for the block number".to_owned(),
                    ))?;
            let new_block = Block {
                header: new_block_header,
                body: new_block_body,
            };
            let (signature, recovery_id) = if let Some(recovered_sig) = conn_l2_state
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
                let Some(secret_key) = conn_l2_state.commiter_key else {
                    return Err(RLPxError::InternalError(
                        "Secret key is not set for based connection".to_string(),
                    ));
                };
                let (recovery_id, signature) = secp256k1::SECP256K1
                    .sign_ecdsa_recoverable(
                        &SecpMessage::from_digest(new_block.hash().to_fixed_bytes()),
                        &secret_key,
                    )
                    .serialize_compact();
                let recovery_id: [u8; 4] = recovery_id.to_i32().to_be_bytes();
                (signature, recovery_id)
            };

            (signature, recovery_id, new_block)
        };

        conn.send(Message::NewBlock(NewBlockMessage {
            block: new_block,
            signature,
            recovery_id,
        }))
        .await?;
    }

    // FIXME: Check if we can avoid this if we add it as a method
    // for l2 state.
    let Some(ref mut conn_l2_state) = conn.l2_state else {
        return Err(RLPxError::IncompatibleProtocol)
    };

    conn_l2_state.latest_block_sent = latest_block_number;

    Ok(())
}

pub async fn should_process_new_block<S: AsyncWrite + AsyncRead + std::marker::Unpin>(conn: &mut RLPxConnection<S>, msg: &NewBlockMessage) -> Result<bool, RLPxError> {
    if !conn.blockchain.is_synced() {
        debug!("Not processing new block, blockchain is not synced");
        return Ok(false);
    }

    // FIXME: Avoid clone & unwrap
    if conn.l2_state.clone().unwrap().latest_block_added >= msg.block.header.number
        // FIXME: Avoid clone & unwrap
        || conn.l2_state.clone().unwrap().blocks_on_queue.contains_key(&msg.block.header.number)
    {
        debug!(
            "Block {} received by peer already stored, ignoring it",
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
                &conn.node,
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
    // FIXME: Avoid clone & unwrap
    conn.l2_state.clone().unwrap().store_rollup
        .store_signature_by_block(block_hash, signature)
        .await?;
    Ok(true)
}

fn validate_signature(_recovered_lead_sequencer: Address) -> bool {
    // Until the RPC module can be included in the P2P crate, we skip the validation
    true
}


async fn send_sealed_batch<S: AsyncWrite + AsyncRead + std::marker::Unpin>(conn: &mut RLPxConnection<S>) -> Result<(), RLPxError> {
    {
        // FIXME: Avoid clone + unwrap.
        let next_batch_to_send = conn.l2_state.clone().unwrap().latest_batch_sent + 1;
        // FIXME: Avoid clone + unwrap.
        if !conn
            .l2_state
            .clone()
            .unwrap()
            .store_rollup
            .contains_batch(&next_batch_to_send)
            .await?
        {
            return Ok(());
        }
        // FIXME: Avoid clone + unwrap
        let Some(batch) = conn.l2_state.clone().unwrap().store_rollup.get_batch(next_batch_to_send).await? else {
            return Ok(());
        };

        // FIXME: Avoid clone + unwrap
        let (signature, recovery_id) = if let Some(recovered_sig) = conn
            .l2_state
            .clone()
            .unwrap()
            .store_rollup
            .get_signature_by_batch(next_batch_to_send)
            .await?
        {
            let mut signature = [0u8; 64];
            let mut recovery_id = [0u8; 4];
            signature.copy_from_slice(&recovered_sig[..64]);
            recovery_id.copy_from_slice(&recovered_sig[64..68]);
            (signature, recovery_id)
        } else {
            // FIXME: Avoid clone + unwrap, try to avoid some check
            let Some(secret_key) = conn.l2_state.clone().unwrap().commiter_key else {
                return Err(RLPxError::InternalError(
                    "Secret key is not set for based connection".to_string(),
                ));
            };
            let (recovery_id, signature) = secp256k1::SECP256K1
                .sign_ecdsa_recoverable(
                    &SecpMessage::from_digest(get_hash_batch_sealed(&batch)),
                    &secret_key,
                )
                .serialize_compact();
            let recovery_id: [u8; 4] = recovery_id.to_i32().to_be_bytes();
            (signature, recovery_id)
        };

        let msg = Message::BatchSealed(BatchSealedMessage {
            batch,
            signature,
            recovery_id,
        });
        conn.send(msg).await?;
        // FIXME: Avoid clone + unwrap, try to avoid some check
        let Some(ref mut l2_state) = conn.l2_state else {
            return Err(RLPxError::IncompatibleProtocol);
        };
        l2_state.latest_batch_sent += 1;
        Ok(())
    }
}

pub async fn process_batch_sealed<S: AsyncWrite + AsyncRead + std::marker::Unpin>(conn: &mut RLPxConnection<S>, msg: &BatchSealedMessage) -> Result<(), RLPxError> {
    // FIXME: Avoid unwrap + clone
    conn.l2_state.clone().unwrap().store_rollup.seal_batch(msg.batch.clone()).await?;
    info!(
        "Sealed batch {} with blocks from {} to {}",
        msg.batch.number, msg.batch.first_block, msg.batch.last_block
    );
    Ok(())
}
