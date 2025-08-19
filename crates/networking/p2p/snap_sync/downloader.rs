use keccak_hash::H256;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CallResponse, GenServer, GenServerHandle},
};

use crate::{
    kademlia::PeerChannels,
    rlpx::{
        Message as RLPxMessage,
        connection::server::{CallMessage, OutMessage},
        eth::blocks::{GetBlockHeaders, HashOrNumber},
    },
};

use tracing::debug;

pub struct Downloader {
    peer_id: H256,
    peer_channels: PeerChannels,
}

impl Downloader {
    pub fn new(peer_id: H256, peer_channels: PeerChannels) -> Self {
        Downloader {
            peer_id,
            peer_channels,
        }
    }
}

#[derive(Clone)]
pub enum DownloaderCallRequest {
    CurrentHead { sync_head: H256 },
}

#[derive(Clone)]
pub enum DownloaderCallResponse {
    NotFound,         // Whatever we were looking for was not found
    CurrentHead(u64), // The sync head block number
}

impl GenServer for Downloader {
    type Error = ();
    type CallMsg = DownloaderCallRequest;
    type CastMsg = Unused;
    type OutMsg = DownloaderCallResponse;

    async fn handle_call(
        &mut self,
        message: Self::CallMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CallResponse<Self> {
        match message {
            DownloaderCallRequest::CurrentHead { sync_head } => {
                let request_id = rand::random();
                let request = RLPxMessage::GetBlockHeaders(GetBlockHeaders {
                    id: request_id,
                    startblock: HashOrNumber::Hash(sync_head),
                    limit: 1,
                    skip: 0,
                    reverse: false,
                });

                let peer_receiving_end = self.peer_channels.receiver.clone();
                match self
                    .peer_channels
                    .connection
                    .call(CallMessage::BackendMessage(
                        peer_receiving_end,
                        request.clone(),
                    ))
                    .await
                {
                    Ok(Ok(OutMessage::BlockHeadersRequest(response))) => {
                        if response.id == request_id && !response.block_headers.is_empty() {
                            let sync_head_number = response.block_headers.last().unwrap().number;
                            return CallResponse::Reply(DownloaderCallResponse::CurrentHead(
                                sync_head_number,
                            ));
                        } else {
                            debug!("Received unexpected response from peer {}", self.peer_id);
                        }
                    }
                    _ => {
                        debug!("Error requesting current head to {}", self.peer_id)
                    }
                }

                // self.peer_channels
                //     .connection
                //     .cast(CastMessage::BackendMessage(request.clone()))
                //     .await
                //     .map_err(|e| format!("Failed to send message to peer {}: {e}", self.peer_id))
                //     .unwrap(); // TODO: handle unwrap

                // let peer_id = self.peer_id;
                // match tokio::time::timeout(Duration::from_millis(100), async move {
                //     self.peer_channels.receiver.lock().await.recv().await
                // })
                // .await
                // {
                //     Ok(Some(RLPxMessage::BlockHeaders(BlockHeaders { id, block_headers }))) => {
                //         if id == request_id && !block_headers.is_empty() {
                //             let sync_head_number = block_headers.last().unwrap().number;
                //             debug!(
                //                 "Sync Log 12: Received sync head block headers from peer {peer_id}, sync head number {sync_head_number}",
                //             );
                //             return CallResponse::Reply(DownloaderCallResponse::CurrentHead(
                //                 sync_head_number,
                //             ));
                //         } else {
                //             debug!("Received unexpected response from peer {peer_id}");
                //         }
                //     }
                //     Ok(None) => {
                //         debug!("Error receiving message from peer {peer_id}")
                //     }
                //     Ok(_other_msgs) => {
                //         debug!("Received unexpected message from peer {peer_id}")
                //     }
                //     Err(_err) => {
                //         debug!("Timeout while waiting for sync head from {peer_id}")
                //     }
                // }

                CallResponse::Stop(DownloaderCallResponse::NotFound)
            }
        }
    }
}
