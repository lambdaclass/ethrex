use std::{sync::Arc, time::Duration};

use ethrex_core::{
    types::{BlockBody, BlockHeader},
    H256,
};
use tokio::sync::{mpsc, Mutex};

use crate::{
    rlpx::eth::blocks::{
        BlockBodies, BlockHeaders, GetBlockBodies, GetBlockHeaders, BLOCK_HEADER_LIMIT,
    },
    RLPxMessage,
};

pub const PEER_REPLY_TIMOUT: Duration = Duration::from_secs(45);
pub const MAX_MESSAGES_IN_PEER_CHANNEL: usize = 25;

#[derive(Debug, Clone)]
/// Holds the respective sender and receiver ends of the communication channels bewteen the peer data and its active connection
pub struct PeerChannels {
    sender: mpsc::Sender<RLPxMessage>,
    receiver: Arc<Mutex<mpsc::Receiver<RLPxMessage>>>,
}

impl PeerChannels {
    /// Sets up the communication channels for the peer
    /// Returns the channel endpoints to send to the active connection's listen loop
    pub(crate) fn create() -> (Self, mpsc::Sender<RLPxMessage>, mpsc::Receiver<RLPxMessage>) {
        let (sender, connection_receiver) =
            mpsc::channel::<RLPxMessage>(MAX_MESSAGES_IN_PEER_CHANNEL);
        let (connection_sender, receiver) =
            mpsc::channel::<RLPxMessage>(MAX_MESSAGES_IN_PEER_CHANNEL);
        (
            Self {
                sender,
                receiver: Arc::new(Mutex::new(receiver)),
            },
            connection_sender,
            connection_receiver,
        )
    }

    /// Requests block headers from the peer
    /// Returns the response message or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - The response timed out
    /// - The response was empty or not valid
    pub async fn request_block_headers(&self, start: H256) -> Option<Vec<BlockHeader>> {
        let request_id = rand::random();
        let request = RLPxMessage::GetBlockHeaders(GetBlockHeaders {
            id: request_id,
            startblock: start.into(),
            limit: BLOCK_HEADER_LIMIT,
            skip: 0,
            reverse: false,
        });
        self.sender.send(request).await.ok()?;
        let mut receiver = self.receiver.lock().await;
        let block_headers = tokio::time::timeout(PEER_REPLY_TIMOUT, async move {
            loop {
                match receiver.recv().await {
                    Some(RLPxMessage::BlockHeaders(BlockHeaders { id, block_headers }))
                        if id == request_id =>
                    {
                        return Some(block_headers)
                    }
                    // Ignore replies that don't match the expected id (such as late responses)
                    Some(_) => continue,
                    None => return None,
                }
            }
        })
        .await
        .ok()??;
        (!block_headers.is_empty()).then_some(block_headers)
    }

    /// Requests block headers from the peer
    /// Returns the response message or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - The response timed out
    /// - The response was empty or not valid
    pub async fn request_block_bodies(&self, block_hashes: Vec<H256>) -> Option<Vec<BlockBody>> {
        let block_hashes_len = block_hashes.len();
        let request_id = rand::random();
        let request = RLPxMessage::GetBlockBodies(GetBlockBodies {
            id: request_id,
            block_hashes,
        });
        self.sender.send(request).await.ok()?;
        let mut receiver = self.receiver.lock().await;
        let block_bodies = tokio::time::timeout(PEER_REPLY_TIMOUT, async move {
            loop {
                match receiver.recv().await {
                    Some(RLPxMessage::BlockBodies(BlockBodies { id, block_bodies }))
                        if id == request_id =>
                    {
                        return Some(block_bodies)
                    }
                    // Ignore replies that don't match the expected id (such as late responses)
                    Some(_) => continue,
                    None => return None,
                }
            }
        })
        .await
        .ok()??;
        // Check that the response is not empty and does not contain more bodies than the ones requested
        (!block_bodies.is_empty() && block_bodies.len() <= block_hashes_len).then_some(block_bodies)
    }
}
