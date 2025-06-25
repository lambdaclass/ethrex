use crate::rlpx::{connection::RLPxConnection, error::RLPxError, message::Message};
use crate::rlpx::l2::messages::{NewBlockMessage, BatchSealedMessage};
use ethrex_common::types::Block;
use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};
use secp256k1::{Message as SecpMessage, SecretKey};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::debug;
#[derive(Debug, Clone)]
pub struct L2ConnState {
    pub latest_block_sent: u64,
    pub latest_batch_sent: u64,
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
