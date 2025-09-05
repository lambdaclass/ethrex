use std::{sync::Arc, time::Duration};

use ethrex_blockchain::Blockchain;
use ethrex_common::types::{MempoolTransaction, Transaction};
use ethrex_storage::error::StoreError;
use keccak_hash::H256;
use rand::{seq::SliceRandom, thread_rng};
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, send_interval},
};
use tracing::{debug, error, info};

use crate::{
    kademlia::{Kademlia, PeerChannels},
    rlpx::{
        Message,
        connection::server::CastMessage,
        eth::transactions::{NewPooledTransactionHashes, Transactions},
        p2p::{Capability, SUPPORTED_ETH_CAPABILITIES},
    },
};

// Soft limit for the number of transaction hashes sent in a single NewPooledTransactionHashes message as per [the spec](https://github.com/ethereum/devp2p/blob/master/caps/eth.md#newpooledtransactionhashes-0x080)
const NEW_POOLED_TRANSACTION_HASHES_SOFT_LIMIT: usize = 4096;

#[derive(Debug, Clone)]
pub struct TxBroadcaster {
    kademlia: Kademlia,
    blockchain: Arc<Blockchain>,
}

#[derive(Debug, Clone)]
pub enum InMessage {
    BroadcastTxs,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

impl TxBroadcaster {
    pub async fn spawn(
        kademlia: Kademlia,
        blockchain: Arc<Blockchain>,
    ) -> Result<(), TxBroadcasterError> {
        info!("Starting Transaction Broadcaster");

        let state = TxBroadcaster {
            kademlia,
            blockchain,
        };

        let server = state.clone().start();

        send_interval(
            Duration::from_secs(1),
            server.clone(),
            InMessage::BroadcastTxs,
        );

        Ok(())
    }

    async fn broadcast_txs(&self) -> Result<(), TxBroadcasterError> {
        let txs_to_broadcast = self
            .blockchain
            .mempool
            .get_txs_for_broadcast()
            .map_err(|_| TxBroadcasterError::Broadcast)?;
        if txs_to_broadcast.is_empty() {
            debug!("No transactions to broadcast");
            return Ok(());
        }
        let peers = self.kademlia.get_peer_channels_with_capabilities(&[]).await;
        let peer_sqrt = (peers.len() as f64).sqrt();

        let full_txs = txs_to_broadcast
            .clone()
            .into_iter()
            .map(|tx| tx.transaction().clone())
            .filter(|tx| !matches!(tx, Transaction::EIP4844Transaction { .. }))
            .collect::<Vec<Transaction>>();

        let blob_txs = txs_to_broadcast
            .iter()
            .filter(|tx| matches!(tx.transaction(), Transaction::EIP4844Transaction { .. }))
            .cloned()
            .collect::<Vec<MempoolTransaction>>();

        let txs_message = Message::Transactions(Transactions {
            transactions: full_txs.clone(),
        });

        let mut shuffled_peers = peers.clone();
        shuffled_peers.shuffle(&mut thread_rng());

        let (peers_to_send_full_txs, peers_to_send_hashes) =
            shuffled_peers.split_at(peer_sqrt.ceil() as usize);

        for (peer_id, mut peer_channels, capabilities) in peers_to_send_full_txs.iter().cloned() {
            // If a peer is selected to receive the full transactions, we don't send the blob transactions, since they only require to send the hashes
            peer_channels.connection.cast(CastMessage::BackendMessage(
                txs_message.clone(),
            )).await.unwrap_or_else(|err| {
                error!(peer_id = %format!("{:#x}", peer_id), err = ?err, "Failed to send transactions");
            });
            self.send_tx_hashes(blob_txs.clone(), capabilities, &mut peer_channels, peer_id)
                .await?;
        }
        for (peer_id, mut peer_channels, capabilities) in peers_to_send_hashes.iter().cloned() {
            // If a peer is not selected to receive the full transactions, we only send the hashes of all transactions (including blob transactions)
            self.send_tx_hashes(
                txs_to_broadcast.clone(),
                capabilities,
                &mut peer_channels,
                peer_id,
            )
            .await?;
        }
        self.blockchain.mempool.clear_broadcasted_txs();
        Ok(())
    }

    async fn send_tx_hashes(
        &self,
        txs: Vec<MempoolTransaction>,
        capabilities: Vec<Capability>,
        peer_channels: &mut PeerChannels,
        peer_id: H256,
    ) -> Result<(), TxBroadcasterError> {
        send_tx_hashes(txs, capabilities, peer_channels, peer_id, &self.blockchain).await
    }
}

pub async fn send_tx_hashes(
    txs: Vec<MempoolTransaction>,
    capabilities: Vec<Capability>,
    peer_channels: &mut PeerChannels,
    peer_id: H256,
    blockchain: &Arc<Blockchain>,
) -> Result<(), TxBroadcasterError> {
    if SUPPORTED_ETH_CAPABILITIES
        .iter()
        .any(|cap| capabilities.contains(cap))
    {
        for tx_chunk in txs.chunks(NEW_POOLED_TRANSACTION_HASHES_SOFT_LIMIT) {
            let tx_count = tx_chunk.len();
            let mut txs_to_send = Vec::with_capacity(tx_count);
            for tx in tx_chunk {
                txs_to_send.push((**tx).clone());
            }
            let hashes_message = Message::NewPooledTransactionHashes(
                NewPooledTransactionHashes::new(txs_to_send, blockchain)?,
            );
            peer_channels.connection.cast(CastMessage::BackendMessage(
                    hashes_message.clone(),
                )).await.unwrap_or_else(|err| {
                    error!(peer_id = %format!("{:#x}", peer_id), err = ?err, "Failed to send transactions hashes");
                });
        }
    }
    Ok(())
}

impl GenServer for TxBroadcaster {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = TxBroadcasterError;

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            Self::CastMsg::BroadcastTxs => {
                debug!(received = "BroadcastTxs");

                let _ = self.broadcast_txs().await.inspect_err(|_| {
                    error!("Failed to broadcast transactions");
                });

                CastResponse::NoReply
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TxBroadcasterError {
    #[error("Failed to broadcast transactions")]
    Broadcast,
    #[error(transparent)]
    StoreError(#[from] StoreError),
}
