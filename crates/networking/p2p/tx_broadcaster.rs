use std::{sync::Arc, time::Duration};

use ethrex_blockchain::Blockchain;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, send_interval},
};
use tracing::{debug, error, info};

use crate::{kademlia::Kademlia, rlpx::connection::server::CastMessage};

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
        let peers = self.kademlia.get_peer_channels(&[]).await;
        for (peer_id, mut peer_channels) in peers {
            peer_channels.connection.cast(CastMessage::SendNewPooledTxHashes(
                txs_to_broadcast.clone(),
            )).await.unwrap_or_else(|err| {
                error!(peer_id = %format!("{:#x}", peer_id), err = ?err, "Failed to send new pooled tx hashes");
            });
        }
        self.blockchain.mempool.clear_broadcasted_txs();
        Ok(())
    }
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
}
