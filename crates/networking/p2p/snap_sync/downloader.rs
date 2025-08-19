use std::time::Duration;

use ethrex_common::types::BlockHeader;
use keccak_hash::H256;
use spawned_concurrency::{
    tasks::{CallResponse, CastResponse, GenServer, GenServerHandle},
};
use tokio::sync::mpsc::Sender;

use crate::{
    kademlia::PeerChannels, peer_handler::BlockRequestOrder, rlpx::{
        connection::server::CastMessage, eth::blocks::{BlockHeaders, GetBlockHeaders, HashOrNumber}, Message as RLPxMessage
    }
};

use tracing::{debug, warn, error};

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

    async fn send_headers_response(
        &self,
        response_channel: Sender<(Vec<BlockHeader>, H256, u64, u64)>,
        headers: Vec<BlockHeader>,
        start_block: u64,
        chunk_limit: u64,
    ) {
        if let Err(_) = response_channel
            .send((headers, self.peer_id, start_block, chunk_limit))
            .await
        {
            error!("[SYNCING] Failed to send headers response to channel"); // TODO: irrecoverable as of now
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

#[derive(Clone)]
pub enum DownloaderCastRequest {
    Headers {
        task_sender: Sender<(Vec<BlockHeader>, H256, u64, u64)>,
        start_block: u64,
        chunk_limit: u64,
    },
}

impl GenServer for Downloader {
    type Error = ();
    type CallMsg = DownloaderCallRequest;
    type CastMsg = DownloaderCastRequest;
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

                self.peer_channels
                    .connection
                    .cast(CastMessage::BackendMessage(request.clone()))
                    .await
                    .map_err(|e| format!("Failed to send message to peer {}: {e}", self.peer_id))
                    .unwrap(); // TODO: handle unwrap

                let peer_id = self.peer_id;
                match tokio::time::timeout(Duration::from_millis(100), async move {
                    self.peer_channels.receiver.lock().await.recv().await
                })
                .await
                {
                    Ok(Some(RLPxMessage::BlockHeaders(BlockHeaders { id, block_headers }))) => {
                        if id == request_id && !block_headers.is_empty() {
                            let sync_head_number = block_headers.last().unwrap().number;
                            debug!(
                                "Sync Log 12: Received sync head block headers from peer {peer_id}, sync head number {sync_head_number}",
                            );
                            return CallResponse::Stop(DownloaderCallResponse::CurrentHead(
                                sync_head_number,
                            ));
                        } else {
                            debug!("Received unexpected response from peer {peer_id}");
                        }
                    }
                    Ok(None) => {
                        debug!("Error receiving message from peer {peer_id}")
                    }
                    Ok(_other_msgs) => {
                        debug!("Received unexpected message from peer {peer_id}")
                    }
                    Err(_err) => {
                        debug!("Timeout while waiting for sync head from {peer_id}")
                    }
                }

                CallResponse::Stop(DownloaderCallResponse::NotFound)
            }
        }
    }

    async fn handle_cast(
            &mut self,
            message: Self::CastMsg,
            _handle: &GenServerHandle<Self>,
        ) -> CastResponse {
            match message {
                DownloaderCastRequest::Headers {
                    task_sender,
                    start_block,
                    chunk_limit,
                } => {
                    debug!("Requesting block headers from peer {}", self.peer_id);
                    let request_id = rand::random();
                    let request = RLPxMessage::GetBlockHeaders(GetBlockHeaders {
                        id: request_id,
                        startblock: HashOrNumber::Number(start_block),
                        limit: chunk_limit,
                        skip: 0,
                        reverse: false,
                    });
                    let mut receiver = self.peer_channels.receiver.lock().await;

                    // FIXME! modify the cast and wait for a `call` version
                    if let Err(err) = self
                        .peer_channels
                        .connection
                        .cast(CastMessage::BackendMessage(request))
                        .await
                    {
                        warn!("Failed to send message to peer: {err:?}");
                        self.send_headers_response(task_sender, vec![], start_block, chunk_limit)
                            .await;
                        return CastResponse::Stop;
                    };

                    let block_headers = match tokio::time::timeout(Duration::from_secs(2), async move {
                        loop {
                            match receiver.recv().await {
                                Some(RLPxMessage::BlockHeaders(BlockHeaders { id, block_headers }))
                                    if id == request_id =>
                                {
                                    return Some(block_headers);
                                }
                                // Ignore replies that don't match the expected id (such as late responses)
                                Some(_) => continue,
                                None => return None, // EOF
                            }
                        }
                    })
                    .await
                    {
                        Ok(Some(headers)) => headers,
                        _ => {
                            self.send_headers_response(task_sender, vec![], start_block, chunk_limit)
                                .await;
                            return CastResponse::Stop;
                        }
                    };

                    if are_block_headers_chained(&block_headers, &BlockRequestOrder::OldToNew) {
                        self.send_headers_response(
                            task_sender,
                            block_headers,
                            start_block,
                            chunk_limit,
                        )
                        .await;
                    } else {
                        warn!(
                            "[SYNCING] Received invalid headers from peer: {}",
                            self.peer_id
                        );
                        self.send_headers_response(task_sender, vec![], start_block, chunk_limit)
                            .await;
                    }

                    // Nothing to do after completion, stop actor
                    CastResponse::Stop
                }
            }
    }
}

/// Validates the block headers received from a peer by checking that the parent hash of each header
/// matches the hash of the previous one, i.e. the headers are chained
fn are_block_headers_chained(block_headers: &[BlockHeader], order: &BlockRequestOrder) -> bool {
    block_headers.windows(2).all(|headers| match order {
        BlockRequestOrder::OldToNew => headers[1].parent_hash == headers[0].hash(),
        BlockRequestOrder::NewToOld => headers[0].parent_hash == headers[1].hash(),
    })
}
