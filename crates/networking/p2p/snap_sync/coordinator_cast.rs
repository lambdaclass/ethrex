use std::{
    cmp::min,
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::Duration,
};

use ethrex_common::{H256, types::BlockHeader};
use spawned_concurrency::{
    error::GenServerError,
    messages::Unused,
    tasks::{CallResponse, CastResponse, GenServer, GenServerHandle, send_after, send_interval},
};
use tokio::sync::Mutex;
use tracing::{error, info};

use crate::{
    kademlia::Kademlia,
    metrics::METRICS,
    rlpx::{
        self, Message,
        eth::blocks::{BLOCK_HEADER_LIMIT, GetBlockHeaders, HashOrNumber},
    },
    snap_sync::downloader::{Downloader, DownloaderError},
};

#[derive(Debug, thiserror::Error)]
pub enum DownloadCoordinatorError {
    #[error("Spawned GenServer Error")]
    GenServerError(#[from] GenServerError),
    #[error("Internal error, this is a bug: {0}")]
    InternalError(#[from] InternalError),
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum InternalError {
    #[error("The request incoming from the coordinator is not a block headers request")]
    InvalidDownloaderRequest,
}

#[derive(Debug, Clone)]
pub struct DownloadCoordinatorState {
    downloaders: Arc<Mutex<BTreeMap<H256, bool>>>,
    pending_downloads: Arc<Mutex<VecDeque<Message>>>,
    downloaded_headers: Arc<Mutex<Vec<BlockHeader>>>,
    kademlia: Kademlia,
}

impl DownloadCoordinatorState {
    pub async fn new(kademlia: Kademlia) -> Self {
        info!("Creating DownloadCoordinatorState");

        let peers_table = kademlia.peers.clone();
        let peers_table = peers_table.lock().await;

        let current_peers = peers_table.keys().map(|peer_id| (*peer_id, true));

        let initial_downloaders = BTreeMap::from_iter(current_peers);

        *METRICS.total_downloaders.lock().await = initial_downloaders.len() as u64;
        *METRICS.free_downloaders.lock().await = initial_downloaders.len() as u64;

        info!("Initial downloaders: {:?}", initial_downloaders.len());

        Self {
            downloaders: Arc::new(Mutex::new(initial_downloaders)),
            pending_downloads: Arc::new(Mutex::new(VecDeque::new())),
            downloaded_headers: Arc::new(Mutex::new(Vec::new())),
            kademlia,
        }
    }
}

#[expect(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum InMessage {
    // From outside
    DownloadHeaders {
        sync_head: H256,
    },
    DownloadStateTries,
    DownloadStorageTries,
    // From Downloader
    HeadersDownloaded {
        peer_id: H256,
        downloaded_headers: Vec<BlockHeader>,
        download_request: rlpx::Message,
        download_error: Option<DownloaderError>,
    },
    // Internal
    AssignTasks,
    RefreshDownloaders,
}

#[derive(Debug, Clone)]
pub enum OutMessage {}

#[derive(Debug, Clone)]
pub struct DownloadCoordinator;

impl DownloadCoordinator {
    pub async fn spawn(kademlia: Kademlia) -> GenServerHandle<Self> {
        info!("Spawning DownloadCoordinator");

        let state = DownloadCoordinatorState::new(kademlia).await;

        DownloadCoordinator::start(state)
    }
}

impl GenServer for DownloadCoordinator {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type State = DownloadCoordinatorState;
    type Error = DownloadCoordinatorError;

    fn new() -> Self {
        Self {}
    }

    async fn init(
        &mut self,
        handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
        state: Self::State,
    ) -> Result<Self::State, Self::Error> {
        info!("Initializing DownloadCoordinator");

        send_interval(
            Duration::from_secs(5),
            handle.clone(),
            Self::CastMsg::RefreshDownloaders,
        );

        Ok(state)
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
        mut state: Self::State,
    ) -> CastResponse<Self> {
        match message {
            Self::CastMsg::DownloadHeaders { sync_head } => {
                info!("Downloading headers up to sync head: {sync_head}");

                handle_headers_download(&mut state, sync_head).await;

                let _ = handle
                    .clone()
                    .cast(Self::CastMsg::AssignTasks)
                    .await
                    .inspect_err(|err| {
                        error!("Failed to self cast AssignTasks after preparing tasks to download headers: {err}");
                    });

                CastResponse::NoReply(state)
            }
            Self::CastMsg::DownloadStateTries => {
                handle_state_tries_download().await;

                CastResponse::NoReply(state)
            }
            Self::CastMsg::DownloadStorageTries => {
                handle_storage_tries_download().await;

                CastResponse::NoReply(state)
            }
            Self::CastMsg::HeadersDownloaded {
                peer_id,
                downloaded_headers,
                download_request,
                download_error,
            } => {
                let _ = handle_headers_downloaded(
                    &mut state,
                    peer_id,
                    downloaded_headers,
                    download_request,
                    download_error,
                )
                .await
                .inspect_err(|err| {
                    error!("Failed to handle downloaded headers: {err}");
                });

                CastResponse::NoReply(state)
            }
            Self::CastMsg::AssignTasks => {
                handle_assign_tasks(&mut state, handle.clone()).await;

                send_after(
                    Duration::from_secs(1),
                    handle.clone(),
                    Self::CastMsg::AssignTasks,
                );

                CastResponse::NoReply(state)
            }
            Self::CastMsg::RefreshDownloaders => {
                handle_refresh_downloaders(&mut state).await;

                CastResponse::NoReply(state)
            }
        }
    }
}

async fn handle_headers_download(state: &mut DownloadCoordinatorState, sync_head_hash: H256) {
    // TODO: Get sync head number from peers
    let sync_head_block = 800_000;

    *METRICS.headers_to_download.lock().await = sync_head_block;
    *METRICS.sync_head_block.lock().await = sync_head_block;
    *METRICS.sync_head_hash.lock().await = sync_head_hash;

    info!("Preparing tasks to download headers up to sync head: {sync_head_block}");

    // Build headers request
    let headers_requests = (1..=sync_head_block)
        .step_by(BLOCK_HEADER_LIMIT as usize)
        .map(|start_block| {
            rlpx::Message::GetBlockHeaders(GetBlockHeaders {
                id: rand::random(),
                startblock: HashOrNumber::Number(start_block),
                limit: min(sync_head_block - start_block, BLOCK_HEADER_LIMIT),
                skip: 0,
                reverse: false,
            })
        })
        .collect::<VecDeque<_>>();

    *METRICS.tasks_queued.lock().await += headers_requests.len() as u64;

    // Add headers request to pending downloads
    for headers_request in headers_requests {
        state
            .pending_downloads
            .lock()
            .await
            .push_back(headers_request.clone());
    }
}

async fn handle_state_tries_download() {}

async fn handle_storage_tries_download() {}

async fn handle_headers_downloaded(
    state: &mut DownloadCoordinatorState,
    peer_id: H256,
    new_downloaded_headers: Vec<BlockHeader>,
    download_request: rlpx::Message,
    download_error: Option<DownloaderError>, // TODO: Use this error
) -> Result<(), DownloadCoordinatorError> {
    // Mark the downloader as free
    state
        .downloaders
        .lock()
        .await
        .entry(peer_id)
        .and_modify(|free| *free = true);

    *METRICS.free_downloaders.lock().await += 1;

    // Check if we downloaded any headers
    if new_downloaded_headers.is_empty() {
        // If no headers were downloaded, we requeue the request.
        state
            .pending_downloads
            .lock()
            .await
            .push_back(download_request);

        *METRICS.tasks_queued.lock().await += 1;

        return Ok(()); // TODO: Maybe an error here?
    }

    // Extract data from the download request
    let (start_block, assigned_chunk_limit) = {
        let rlpx::Message::GetBlockHeaders(GetBlockHeaders {
            id: _,
            startblock: start_block,
            limit: assigned_chunk_limit,
            ..
        }) = download_request
        else {
            return Err(DownloadCoordinatorError::InternalError(
                InternalError::InvalidDownloaderRequest,
            ));
        };

        let HashOrNumber::Number(start_block) = start_block else {
            return Err(DownloadCoordinatorError::InternalError(
                InternalError::InvalidDownloaderRequest,
            ));
        };

        (start_block, assigned_chunk_limit)
    };

    // Check if we downloaded fewer headers than requested
    let new_headers_count = new_downloaded_headers.len() as u64;
    if new_headers_count < assigned_chunk_limit {
        // Downloader downloaded fewer headers than requested.
        // Queue a new request for the missing headers.
        state
            .pending_downloads
            .lock()
            .await
            .push_back(rlpx::Message::GetBlockHeaders(GetBlockHeaders {
                id: rand::random(),
                startblock: HashOrNumber::Number(start_block + new_headers_count),
                limit: assigned_chunk_limit - new_headers_count,
                skip: 0,
                reverse: false,
            }));

        *METRICS.tasks_queued.lock().await += 1;
    }

    // Store the downloaded headers
    state
        .downloaded_headers
        .lock()
        .await
        .extend_from_slice(&new_downloaded_headers);

    *METRICS.downloaded_headers.lock().await += new_headers_count;
    *METRICS.headers_to_download.lock().await -= new_headers_count;

    Ok(())
}

async fn handle_refresh_downloaders(state: &mut DownloadCoordinatorState) {
    let mut downloaders = state.downloaders.lock().await;

    let peers_table = state.kademlia.peers.clone();
    let peers_table = peers_table.lock().await;

    let current_peers = peers_table.keys().map(|peer_id| (*peer_id, true));

    // Update downloaders with current peers
    for (peer_id, _) in current_peers {
        downloaders.entry(peer_id).or_insert(true);
    }

    // Remove downloaders that are no longer in the peer table
    downloaders.retain(|peer_id, _| peers_table.contains_key(peer_id));

    *METRICS.total_downloaders.lock().await = downloaders.len() as u64;
    *METRICS.free_downloaders.lock().await =
        downloaders.values().filter(|&&free| free).count() as u64;
}

async fn handle_assign_tasks(
    state: &mut DownloadCoordinatorState,
    self_handle: GenServerHandle<DownloadCoordinator>,
) {
    let mut downloaders = state.downloaders.lock().await;

    for (free_peer_id, free) in downloaders.iter_mut().filter(|&(_, &mut free)| free) {
        if let Some(free_peer_data) = state.kademlia.peers.lock().await.get(free_peer_id) {
            // Assign tasks to the downloader
            let Some(task) = state.pending_downloads.lock().await.pop_front() else {
                // TODO: Handle case where no tasks are available
                return;
            };

            let _ = Downloader::spawn_as_headers_downloader(
                free_peer_data.clone(),
                self_handle.clone(),
                task,
            );

            *free = false; // Mark the downloader as busy

            *METRICS.free_downloaders.lock().await -= 1;

            info!("Assigned task to peer: {free_peer_id}");
        }
    }
}
