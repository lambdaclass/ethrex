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
        eth::blocks::{BlockHeaders, GetBlockHeaders},
    },
    snap_sync::coordinator::{self, Coordinator},
};

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
        peer: PeerData,
        coordinator_handle: GenServerHandle<Coordinator>,
        download_request: rlpx::Message,
    },
}

pub struct Downloader;

impl Downloader {
    pub fn spawn_as_headers_downloader(
        peer: PeerData,
        coordinator_handle: GenServerHandle<Coordinator>,
        headers_request: rlpx::Message,
    ) -> GenServerHandle<Self> {
        let state = DownloaderState::HeadersDownloader {
            peer,
            coordinator_handle,
            download_request: headers_request,
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
                peer,
                mut coordinator_handle,
                download_request,
            } => {
                let peer_id = peer.node.node_id();

                let (downloaded_headers, download_error) =
                    match handle_headers_download(peer_id, peer.channels, download_request.clone())
                        .await
                    {
                        Ok(headers) => (headers, None),
                        Err(err) => (Vec::new(), Some(err)),
                    };

                let _ = coordinator_handle
                    .cast(coordinator::CastMessage::HeadersDownloaded {
                        peer_id,
                        downloaded_headers,
                        download_request,
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
    peer_channels: Option<PeerChannels>,
    headers_request: rlpx::Message,
) -> Result<Vec<BlockHeader>, DownloaderError> {
    debug!("Requesting block headers from peer {peer_id}");

    let rlpx::Message::GetBlockHeaders(GetBlockHeaders { id: request_id, .. }) = headers_request
    else {
        return Err(InternalError::InvalidDownloaderRequest)?;
    };

    let mut peer_channels = peer_channels.ok_or(DownloaderError::InternalError(
        InternalError::DownloaderHasNoPeerChannels,
    ))?;

    let mut receiver = peer_channels.receiver.lock().await;

    // FIXME! modify the cast and wait for a `call` version
    peer_channels
        .connection
        .cast(CastMessage::BackendMessage(headers_request))
        .await
        .map_err(|err| DownloaderError::FailedToSendMessageToPeer(peer_id, err.to_string()))?;

    let block_headers = tokio::time::timeout(Duration::from_secs(5), async move {
        loop {
            match receiver.recv().await {
                Some(rlpx::Message::BlockHeaders(BlockHeaders {
                    id: response_id,
                    block_headers,
                })) if response_id == request_id => return Some(block_headers),
                // Ignore replies that don't match the expected id (such as late responses)
                Some(_) => continue,
                None => return None, // EOF
            };
        }
    })
    .await
    .map_err(|_elapsed| DownloaderError::BlockHeadersRequestTimedOut(peer_id))?
    .ok_or(DownloaderError::NoBlockHeadersReceived(peer_id))?;

    if are_block_headers_chained(&block_headers) {
        return Err(DownloaderError::BlockHeadersNotChained(peer_id));
    }

    Ok(block_headers)
}

/// Validates the block headers received from a peer by checking that the parent hash of each header
/// matches the hash of the previous one, i.e. the headers are chained
pub fn are_block_headers_chained(block_headers: &[BlockHeader]) -> bool {
    block_headers
        .windows(2)
        .all(|headers| headers[1].parent_hash == headers[0].hash())
}
