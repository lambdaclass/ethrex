use crate::rlpx::based::get_hash_batch_sealed;
use crate::rlpx::l2::messages::{BatchSealed, NewBlock, L2Message};
use crate::rlpx::utils::{get_pub_key, log_peer_error};
use crate::rlpx::{connection::RLPxConnection, error::RLPxError, message::Message};
use ethereum_types::Address;
use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::fork_choice::apply_fork_choice;
use ethrex_common::types::Block;
use ethrex_common::types::batch::Batch;
use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};
use secp256k1::{Message as SecpMessage, SecretKey};
use std::collections::BTreeMap;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;
use tracing::{debug, info};

#[derive(Debug, Clone)]
pub struct L2ConnectedState {
    pub latest_block_sent: u64,
    pub latest_block_added: u64,
    pub latest_batch_sent: u64,
    pub blocks_on_queue: BTreeMap<u64, Block>,
    pub store_rollup: StoreRollup,
    pub committer_key: Option<SecretKey>,
    pub next_block_broadcast: Instant,
    pub next_batch_broadcast: Instant,
}

#[derive(Debug, Clone)]
pub enum L2ConnState {
    Disconnected,
    Connected(L2ConnectedState),
}

impl L2ConnState {
    pub(crate) fn connection_state_mut(&mut self) -> Result<&mut L2ConnectedState, RLPxError> {
        match self {
            Self::Disconnected => Err(RLPxError::L2CapabilityNotNegotiated),
            Self::Connected(conn_state) => Ok(conn_state),
        }
    }
}

fn validate_signature(_recovered_lead_sequencer: Address) -> bool {
    // Until the RPC module can be included in the P2P crate, we skip the validation
    true
}

impl<S: AsyncWrite + AsyncRead + std::marker::Unpin> RLPxConnection<S> {
    pub async fn handle_based_capability_message(
        &mut self,
        msg: L2Message,
    ) -> Result<(), RLPxError> {
        self.l2_state.connection_state_mut()?;
        match msg {
            L2Message::BatchSealed(ref batch_sealed_msg) => {
                if self.should_process_batch_sealed(&batch_sealed_msg).await? {
                    self.process_batch_sealed(&batch_sealed_msg).await?;
                }
            }
            L2Message::NewBlock(ref new_block_msg) => {
                if self.should_process_new_block(&new_block_msg).await? {
                    self.process_new_block(&new_block_msg).await?;
                }
            }
        }
        // for now we broadcast valid messages
        self.broadcast_message(msg.into()).await
    }
    pub async fn send_new_block(&mut self) -> Result<(), RLPxError> {
        // FIXME: Re-add this
        // let latest_block_number = self.storage.get_latest_block_number().await?;
        // for i in self.latest_block_sent + 1..=latest_block_number {
        //     debug!(
        //         "Broadcasting new block, current: {}, last broadcasted: {}",
        //         i, self.latest_block_sent
        //     );

        //     let new_block_body =
        //         self.storage
        //             .get_block_body(i)
        //             .await?
        //             .ok_or(RLPxError::InternalError(
        //                 "Block body not found after querying for the block number".to_owned(),
        //             ))?;
        //     let new_block_header =
        //         self.storage
        //             .get_block_header(i)?
        //             .ok_or(RLPxError::InternalError(
        //                 "Block header not found after querying for the block number".to_owned(),
        //             ))?;
        //     let new_block = Block {
        //         header: new_block_header,
        //         body: new_block_body,
        //     };
        //     let (signature, recovery_id) = if let Some(recovered_sig) = self
        //         .store_rollup
        //         .get_signature_by_block(new_block.hash())
        //         .await?
        //     {
        //         let mut signature = [0u8; 64];
        //         let mut recovery_id = [0u8; 4];
        //         signature.copy_from_slice(&recovered_sig[..64]);
        //         recovery_id.copy_from_slice(&recovered_sig[64..68]);
        //         (signature, recovery_id)
        //     } else {
        //         let Some(secret_key) = self.committer_key else {
        //             return Err(RLPxError::InternalError(
        //                 "Secret key is not set for based connection".to_string(),
        //             ));
        //         };
        //         let (recovery_id, signature) = secp256k1::SECP256K1
        //             .sign_ecdsa_recoverable(
        //                 &SignedMessage::from_digest(new_block.hash().to_fixed_bytes()),
        //                 &secret_key,
        //             )
        //             .serialize_compact();
        //         let recovery_id: [u8; 4] = recovery_id.to_i32().to_be_bytes();
        //         (signature, recovery_id)
        //     };
        //     self.send(Message::NewBlock(NewBlockMessage {
        //         block: new_block,
        //         signature,
        //         recovery_id,
        //     }))
        //     .await?;
        // }
        // self.latest_block_sent = latest_block_number;

        Ok(())
    }

    pub async fn should_process_new_block(&mut self, msg: &NewBlock) -> Result<bool, RLPxError> {
        let l2_state = self.l2_state.connection_state_mut()?;
        if !self.blockchain.is_synced() {
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

        let recovered_lead_sequencer = get_pub_key(
            msg.recovery_id,
            &msg.signature,
            *block_hash.as_fixed_bytes(),
        )
        .map_err(|e| {
            log_peer_error(
                &self.node,
                &format!("Failed to recover lead sequencer: {e}"),
            );
            RLPxError::CryptographyError(e.to_string())
        })?;

        if validate_signature(recovered_lead_sequencer) {
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

    async fn should_process_batch_sealed(&mut self, msg: &BatchSealed) -> Result<bool, RLPxError> {
        let l2_state = self.l2_state.connection_state_mut()?;
        if !self.blockchain.is_synced() {
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
            debug!(
                "Not processing batch {} because the last block {} is not added yet",
                msg.batch.number, msg.batch.last_block
            );
            return Ok(false);
        }

        let hash = get_hash_batch_sealed(&msg.batch);

        let recovered_lead_sequencer =
            get_pub_key(msg.recovery_id, &msg.signature, hash).map_err(|e| {
                log_peer_error(
                    &self.node,
                    &format!("Failed to recover lead sequencer: {e}"),
                );
                RLPxError::CryptographyError(e.to_string())
            })?;

        if validate_signature(recovered_lead_sequencer) {
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
    pub async fn process_new_block(&mut self, msg: &NewBlock) -> Result<(), RLPxError> {
        let l2_state = self.l2_state.connection_state_mut()?;
        l2_state
            .blocks_on_queue
            .entry(msg.block.header.number)
            .or_insert_with(|| msg.block.clone());

        let mut next_block_to_add = l2_state.latest_block_added + 1;
        while let Some(block) = l2_state.blocks_on_queue.remove(&next_block_to_add) {
            // This check is necessary if a connection to another peer already applied the block but this connection
            // did not register that update.
            if let Ok(Some(_)) = self.storage.get_block_body(next_block_to_add).await {
                l2_state.latest_block_added = next_block_to_add;
                next_block_to_add += 1;
                continue;
            }
            self.blockchain.add_block(&block).await.inspect_err(|e| {
                log_peer_error(
                    &self.node,
                    &format!(
                        "Error adding new block {} with hash {:?}, error: {e}",
                        block.header.number,
                        block.hash()
                    ),
                );
            })?;
            let block_hash = block.hash();

            apply_fork_choice(&self.storage, block_hash, block_hash, block_hash)
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

    pub async fn send_sealed_batch(&mut self) -> Result<(), RLPxError> {
        let (batch, signature, recovery_id) = {
            let l2_state = self.l2_state.connection_state_mut()?;
            let next_batch_to_send = l2_state.latest_batch_sent + 1;
            if l2_state
                .store_rollup
                .contains_batch(&next_batch_to_send)
                .await?
            {
                return Ok(());
            }
            let Some(batch) = l2_state.store_rollup.get_batch(next_batch_to_send).await? else {
                return Ok(());
            };

            let (signature, recovery_id) = if let Some(recovered_sig) = l2_state
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
                let Some(secret_key) = l2_state.committer_key else {
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
            (batch, signature, recovery_id)
        };

        let msg = BatchSealed {
            batch,
            signature,
            recovery_id,
        };

        self.send(msg.into()).await?;
        self.l2_state.connection_state_mut()?.latest_batch_sent += 1;
        Ok(())
    }

    async fn process_batch_sealed(&mut self, msg: &BatchSealed) -> Result<(), RLPxError> {
        let l2_state = self.l2_state.connection_state_mut()?;
        l2_state.store_rollup.seal_batch(msg.batch.clone()).await?;
        info!(
            "Sealed batch {} with blocks from {} to {}",
            msg.batch.number, msg.batch.first_block, msg.batch.last_block
        );
        Ok(())
    }
}
