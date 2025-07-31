use std::time::Duration;

use ethrex_common::{H256, types::BlockHeader};
use spawned_concurrency::{
    messages::Unused,
    tasks::{GenServer, GenServerHandle},
};
use tracing::{debug, warn};

use crate::{
    kademlia::{PeerChannels, PeerData},
    rlpx::{
        self,
        connection::server::CastMessage,
        eth::blocks::{BlockHeaders, GetBlockHeaders, HashOrNumber},
    },
    snap_sync::coordinator::{self, Coordinator},
};

const BLOCK_HEADERS_REQUEST_CHUNK_SIZE: u64 = 1024; // Default chunk size for block headers requests

#[derive(Debug, Clone, thiserror::Error)]
pub enum DownloaderError {
    #[error("Failed to send message to peer {0}: {1}")]
    FailedToSendMessageToPeer(H256, String), // TODO: Replace String with GenServerError when Clone is implemented for it.
    #[error("Block headers request timed out, peer {0}")]
    BlockHeadersRequestTimedOut(H256),
    #[error("No block headers received from peer {0}")]
    NoBlockHeadersReceived(H256),
    #[error("Block headers received from peer {0} are not chained")]
    BlockHeadersNotChained(H256),
    #[error("Internal error, this is a bug: {0}")]
    InternalError(#[from] InternalError),
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum InternalError {
    #[error("The request incoming from the coordinator is not a block headers request")]
    InvalidDownloaderRequest,
    #[error("Downloader has no peer channels")]
    DownloaderHasNoPeerChannels,
}

#[derive(Clone)]
pub enum DownloaderState {
    HeadersDownloader {
        peer_id: H256,
        peer_channels: PeerChannels,
        coordinator_handle: GenServerHandle<Coordinator>,
        assigned_start_block: u64,
        assigned_chunk_limit: u64,
    },
}

pub struct Downloader;

impl Downloader {
    pub fn spawn_as_headers_downloader(
        peer_id: H256,
        peer_channels: PeerChannels,
        coordinator_handle: GenServerHandle<Coordinator>,
        assigned_start_block: u64,
        assigned_chunk_limit: u64,
    ) -> GenServerHandle<Self> {
        let state = DownloaderState::HeadersDownloader {
            peer_id,
            peer_channels,
            coordinator_handle,
            assigned_start_block,
            assigned_chunk_limit,
        };

        Downloader::start(state)
    }
}

impl GenServer for Downloader {
    type CallMsg = Unused;
    type CastMsg = Unused;
    type OutMsg = Unused;
    type State = DownloaderState;
    type Error = DownloaderError;

    fn new() -> Self {
        Self {}
    }

    async fn init(
        &mut self,
        _handle: &GenServerHandle<Self>,
        state: Self::State,
    ) -> Result<Self::State, Self::Error> {
        match state.clone() {
            DownloaderState::HeadersDownloader {
                peer_id,
                peer_channels,
                mut coordinator_handle,
                assigned_start_block,
                assigned_chunk_limit,
            } => {
                let (downloaded_headers, download_error) = match handle_headers_download(
                    peer_id,
                    peer_channels,
                    assigned_start_block,
                    assigned_chunk_limit,
                )
                .await
                {
                    Ok(headers) => (headers, None),
                    Err(err) => (Vec::new(), Some(err)),
                };

                let _ = coordinator_handle
                    .cast(coordinator::CastMessage::HeadersDownloaded {
                        peer_id,
                        downloaded_headers,
                        assigned_start_block,
                        assigned_chunk_limit,
                        download_error,
                    })
                    .await
                    .inspect_err(|err| {
                        warn!("Failed to notify coordinator about downloaded headers: {err}");
                    });
            }
        };

        Ok(state)
    }
}

async fn handle_headers_download(
    peer_id: H256,
    mut peer_channels: PeerChannels,
    assigned_start_block: u64,
    assigned_chunk_limit: u64,
) -> Result<Vec<BlockHeader>, DownloaderError> {
    debug!("Requesting block headers from peer {peer_id}");

    let mut receiver = peer_channels.receiver.lock().await;

    let mut cummulative_block_headers: Vec<BlockHeader> = Vec::new();
    let mut current_start_block = assigned_start_block;
    let amount_expected = assigned_chunk_limit;

    // Our first request is eager, we assume the peer can return all requested headers
    // in one go, so we set the chunk size to the assigned chunk limit.
    let mut chunk_size = BLOCK_HEADERS_REQUEST_CHUNK_SIZE;

    while cummulative_block_headers.len() < amount_expected as usize {
        let request_id = rand::random();

        // FIXME! modify the cast and wait for a `call` version
        peer_channels
            .connection
            .cast(CastMessage::BackendMessage(rlpx::Message::GetBlockHeaders(
                GetBlockHeaders {
                    id: request_id,
                    startblock: HashOrNumber::Number(current_start_block),
                    limit: chunk_size,
                    skip: 0,
                    reverse: false,
                },
            )))
            .await
            .map_err(|err| DownloaderError::FailedToSendMessageToPeer(peer_id, err.to_string()))?;

        let block_headers = match receiver.recv().await {
            Some(rlpx::Message::BlockHeaders(BlockHeaders {
                id: response_id,
                block_headers,
            })) if response_id == request_id => block_headers,
            // Ignore replies that don't match the expected id (such as late responses)
            Some(_) => {
                debug!("Received unexpected BlockHeaders response from peer {peer_id}, ignoring");
                continue;
            }
            None => return Err(DownloaderError::NoBlockHeadersReceived(peer_id)), // EOF
        };

        let retrieved_amount = block_headers.len() as u64;
        cummulative_block_headers.extend(block_headers);
        current_start_block += chunk_size as u64;

        // If we received less headers than the requested chunk size, we assume
        // the peer is limited in the return size, so we adjust the chunk size
        chunk_size = retrieved_amount;
    }

    if !are_block_headers_chained(&cummulative_block_headers) {
        return Err(DownloaderError::BlockHeadersNotChained(peer_id));
    }

    Ok(cummulative_block_headers)
}

/// Validates the block headers received from a peer by checking that the parent hash of each header
/// matches the hash of the previous one, i.e. the headers are chained
pub fn are_block_headers_chained(block_headers: &[BlockHeader]) -> bool {
    block_headers
        .windows(2)
        .all(|headers| headers[1].parent_hash == headers[0].hash())
}
