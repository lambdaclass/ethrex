use std::{
    cmp::min,
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::{Duration, SystemTime},
};

use ethrex_common::{H256, types::BlockHeader};
use rand::seq::SliceRandom;
use spawned_concurrency::{
    error::GenServerError,
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, send_after},
};
use tokio::sync::Mutex;
use tracing::{debug, error, info};

use crate::{
    kademlia::Kademlia,
    metrics::METRICS,
    rlpx::{
        self, Message,
        eth::blocks::{BlockHeaders, GetBlockHeaders, HashOrNumber},
        p2p::SUPPORTED_ETH_CAPABILITIES,
    },
    snap_sync::downloader::{Downloader, DownloaderError},
};

#[derive(Debug, thiserror::Error)]
pub enum CoordinatorError {
    #[error("Spawned GenServer Error")]
    GenServerError(#[from] GenServerError),
    #[error("Internal error, this is a bug: {0}")]
    InternalError(#[from] InternalError),
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum InternalError {
    #[error("The request incoming from the coordinator is not a block headers request")]
    InvalidDownloaderRequest,
    #[error("Received download from peer {0} but Coordinator is asleep")]
    UnexpectedDownloadRequest(H256),
}

#[derive(Debug, Clone)]
pub enum CoordinatorState {
    Syncing {
        sync_head_number: Arc<Mutex<u64>>,
        start_block_number: u64,
        downloaders: BTreeMap<H256, bool>,

        pending_header_downloads: VecDeque<(u64, u64)>,
        downloaded_headers: Vec<BlockHeader>,

        kademlia: Kademlia,
    },
    Asleep(Kademlia),
}

impl CoordinatorState {
    pub fn new(kademlia: Kademlia) -> Self {
        Self::Asleep(kademlia)
    }

    async fn get_sync_head_block_number(&mut self, sync_head_hash: H256) {
        match self {
            CoordinatorState::Syncing {
                sync_head_number,
                start_block_number: _,
                downloaders: _,
                pending_header_downloads: _,
                downloaded_headers: _,
                kademlia,
            } => {
                debug!("Retrieving sync head block number from peers");

                let peers_table = kademlia.peers.clone();
                let peers_table = peers_table.lock().await;

                let sync_head_number_retrieval_start = SystemTime::now();

                while *sync_head_number.lock().await == 0 {
                    for (peer_id, peer_data) in peers_table.clone() {
                        let Some(mut peer_channels) = peer_data.channels else {
                            debug!("Peer {peer_id} has no channels, skipping");
                            continue;
                        };

                        let sync_head_number = sync_head_number.clone();

                        tokio::spawn(async move {
                            let request_id = rand::random();

                            let request = rlpx::Message::GetBlockHeaders(GetBlockHeaders {
                                id: request_id,
                                startblock: HashOrNumber::Hash(sync_head_hash),
                                limit: 1,
                                skip: 0,
                                reverse: false,
                            });

                            let _ = peer_channels
                                .connection
                                .cast(rlpx::connection::server::CastMessage::BackendMessage(
                                    request.clone(),
                                ))
                                .await;

                            match tokio::time::timeout(Duration::from_secs(5), async move {
                                peer_channels
                                    .receiver
                                    .lock()
                                    .await
                                    .recv()
                                    .await
                                    .expect("Failed to receive message from peer")
                            })
                            .await
                            {
                                Ok(rlpx::Message::BlockHeaders(BlockHeaders {
                                    id,
                                    block_headers,
                                })) => {
                                    if id == request_id && !block_headers.is_empty() {
                                        let mut sync_head_number = sync_head_number.lock().await;

                                        if *sync_head_number == 0 {
                                            *sync_head_number = block_headers
                                                .last()
                                                .expect("No block headers received from peer")
                                                .number;
                                        }
                                    }
                                }
                                Ok(_other_msgs) => {
                                    debug!("Received unexpected message from peer {peer_id}");
                                }
                                Err(_err) => {
                                    debug!("Timeout while waiting for sync head from {peer_id}");
                                }
                            }
                        });
                    }
                }

                let sync_head_number_retrieval_elapsed = sync_head_number_retrieval_start
                    .elapsed()
                    .expect("Failed to get elapsed time");

                info!("Sync head block number retrieved");

                *METRICS.time_to_retrieve_sync_head_block.lock().await =
                    Some(sync_head_number_retrieval_elapsed);
                *METRICS.sync_head_hash.lock().await = sync_head_hash;
            }
            CoordinatorState::Asleep(..) => {}
        }
    }

    async fn prepare_download_tasks(&mut self) {
        match self {
            CoordinatorState::Syncing {
                sync_head_number,
                start_block_number,
                downloaders: _,
                pending_header_downloads,
                downloaded_headers: _,
                kademlia: _,
            } => {
                let sync_head_block = *sync_head_number.lock().await;
                let start_block = *start_block_number;

                info!("Preparing tasks to download headers up to sync head: {sync_head_block}",);

                let block_count = sync_head_block + 1 - start_block;

                let chunk_count = if block_count < 800_u64 { 1 } else { 800_u64 };

                let chunk_limit = block_count / chunk_count as u64;

                let mut tasks_queue_not_started = VecDeque::<(u64, u64)>::new();

                for i in 0..(chunk_count as u64) {
                    tasks_queue_not_started.push_back((i * chunk_limit + start_block, chunk_limit));
                }

                // Push the reminder
                if block_count % chunk_count as u64 != 0 {
                    tasks_queue_not_started.push_back((
                        chunk_count as u64 * chunk_limit + start_block,
                        block_count % chunk_count as u64,
                    ));
                }

                *pending_header_downloads = tasks_queue_not_started;
            }
            CoordinatorState::Asleep(..) => {}
        }
    }

    async fn handle_state_tries_download(&mut self) {}

    async fn handle_storage_tries_download(&mut self) {}

    async fn handle_headers_downloaded(
        &mut self,
        peer_id: H256,
        new_downloaded_headers: Vec<BlockHeader>,
        assigned_start_block: u64,
        assigned_chunk_limit: u64,
        download_error: Option<DownloaderError>, // TODO: Use this error
    ) -> Result<(), CoordinatorError> {
        match self {
            CoordinatorState::Syncing {
                sync_head_number: _,
                start_block_number: _,
                downloaders,
                pending_header_downloads,
                downloaded_headers,
                kademlia: _,
            } => {
                debug!(
                    "Received {} headers from peer {peer_id}",
                    new_downloaded_headers.len(),
                );

                // Mark the downloader as free
                downloaders.entry(peer_id).and_modify(|free| *free = true);

                // Check if we downloaded any headers
                if new_downloaded_headers.is_empty() {
                    // If no headers were downloaded, we requeue the request.
                    pending_header_downloads
                        .push_back((assigned_start_block, assigned_chunk_limit));

                    return Ok(()); // TODO: Maybe an error here?
                }

                // Check if we downloaded fewer headers than requested
                let new_headers_count = new_downloaded_headers.len() as u64;
                if new_headers_count < assigned_chunk_limit {
                    // Downloader downloaded fewer headers than requested.
                    // Queue a new request for the missing headers.
                    pending_header_downloads.push_back((
                        assigned_start_block + new_headers_count,
                        assigned_chunk_limit - new_headers_count,
                    ));
                }

                // Store the downloaded headers
                downloaded_headers.extend_from_slice(&new_downloaded_headers);

                Ok(())
            }
            CoordinatorState::Asleep(..) => {
                Err(InternalError::UnexpectedDownloadRequest(peer_id).into())
            }
        }
    }

    async fn handle_refresh_downloaders(&mut self) {
        match self {
            CoordinatorState::Syncing {
                sync_head_number: _,
                start_block_number: _,
                downloaders,
                pending_header_downloads: _,
                downloaded_headers: _,
                kademlia,
            } => {
                debug!("Refreshing downloaders");

                let peers_table = kademlia.peers.clone();
                let peers_table = peers_table.lock().await;

                // Update downloaders with current peers
                for peer_id in peers_table.keys().cloned() {
                    downloaders.entry(peer_id).or_insert(true);
                }

                // Remove downloaders that are no longer in the peer table
                downloaders.retain(|peer_id, _| peers_table.contains_key(peer_id));
            }
            CoordinatorState::Asleep(..) => {}
        }
    }

    async fn handle_assign_download_tasks(&mut self, self_handle: GenServerHandle<Coordinator>) {
        match self {
            CoordinatorState::Syncing {
                sync_head_number,
                start_block_number,
                downloaders,
                pending_header_downloads,
                downloaded_headers,
                kademlia,
            } => {
                let peer_channels = kademlia
                    .get_peer_channels(&SUPPORTED_ETH_CAPABILITIES)
                    .await;

                for (peer_id, _peer_channels) in &peer_channels {
                    if downloaders.contains_key(peer_id) {
                        // Peer is already in the downloaders list, skip it
                        continue;
                    }

                    downloaders.insert(*peer_id, true);

                    debug!("{peer_id} added as downloader");
                }

                let mut free_downloaders = downloaders
                    .clone()
                    .into_iter()
                    .filter(|(_downloader_id, downloader_is_free)| *downloader_is_free)
                    .collect::<Vec<_>>();

                if free_downloaders.is_empty() {
                    return;
                }

                free_downloaders.shuffle(&mut rand::rngs::OsRng);

                for (free_peer_id, _) in free_downloaders {
                    let Some(free_downloader_channels) =
                        peer_channels.iter().find_map(|(peer_id, peer_channels)| {
                            peer_id.eq(&free_peer_id).then_some(peer_channels.clone())
                        })
                    else {
                        // The free downloader is not a peer of us anymore.
                        debug!(
                            "Downloader {free_peer_id} is not a peer anymore, removing it from the downloaders list"
                        );
                        downloaders.remove(&free_peer_id);
                        continue;
                    };

                    let Some((start_block, chunk_limit)) = pending_header_downloads.pop_front()
                    else {
                        if downloaded_headers.len() as u64
                            >= (*sync_head_number.lock().await - *start_block_number)
                        {
                            info!("All headers downloaded successfully");
                            break;
                        }

                        continue;
                    };

                    let _ = Downloader::spawn_as_headers_downloader(
                        free_peer_id,
                        free_downloader_channels,
                        self_handle.clone(),
                        start_block,
                        chunk_limit,
                    );

                    downloaders
                        .entry(free_peer_id)
                        .and_modify(|downloader_is_free| {
                            *downloader_is_free = false; // mark the downloader as busy
                        });
                }
            }
            CoordinatorState::Asleep(..) => {}
        }
    }

    async fn handle_update_metrics(&self) {
        match self {
            CoordinatorState::Syncing {
                sync_head_number,
                start_block_number,
                downloaders,
                pending_header_downloads,
                downloaded_headers,
                kademlia,
            } => {
                *METRICS.headers_to_download.lock().await = *sync_head_number.lock().await;
                *METRICS.sync_head_block.lock().await = *sync_head_number.lock().await;
                *METRICS.header_downloads_tasks_queued.lock().await =
                    pending_header_downloads.len() as u64;
                *METRICS.total_header_downloaders.lock().await = downloaders.len() as u64;
                *METRICS.free_header_downloaders.lock().await =
                    downloaders.values().filter(|&&free| free).count() as u64;
                *METRICS.downloaded_headers.lock().await = downloaded_headers.len() as u64;
            }
            CoordinatorState::Asleep(kademlia) => {}
        }
    }
}

#[derive(Debug, Clone)]
pub enum CastMessage {
    // External
    SyncToHead {
        from_block_number: u64,
        to_block_head: H256,
    },
    // Internal
    DownloadHeaders,
    DownloadStateTries,
    DownloadStorageTries,
    AssignDownloadTasks,
    // RefreshDownloaders,
    // From Downloader
    HeadersDownloaded {
        peer_id: H256,
        downloaded_headers: Vec<BlockHeader>,
        assigned_start_block: u64,
        assigned_chunk_limit: u64,
        download_error: Option<DownloaderError>,
    },
    // Metrics
    UpdateMetrics,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    DownloadingBlockHeaders { amount: u64 },
}

#[derive(Debug, Clone)]
pub struct Coordinator;

impl Coordinator {
    pub fn spawn(kademlia: Kademlia) -> GenServerHandle<Self> {
        info!("Spawning Coordinator");

        let state = CoordinatorState::new(kademlia);

        Coordinator::start(state)
    }
}

impl GenServer for Coordinator {
    type CallMsg = Unused;
    type CastMsg = CastMessage;
    type OutMsg = OutMessage;
    type State = CoordinatorState;
    type Error = CoordinatorError;

    fn new() -> Self {
        Self {}
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
        mut state: Self::State,
    ) -> CastResponse<Self> {
        match message {
            Self::CastMsg::SyncToHead {
                from_block_number,
                to_block_head,
            } => {
                match state {
                    CoordinatorState::Syncing { .. } => {
                        info!("Received request to sync to head: {to_block_head}");
                        // Coordinator is already syncing, no action needed
                        debug!("Coordinator is already syncing, no action needed");
                    }
                    CoordinatorState::Asleep(kademlia) => {
                        info!("Received request to sync to head: {to_block_head}");

                        let peers_table = kademlia.peers.clone();
                        let peers_table = peers_table.lock().await;

                        let current_peers = peers_table.keys().map(|peer_id| (*peer_id, true));

                        let initial_downloaders = BTreeMap::from_iter(current_peers);

                        drop(peers_table);

                        // Wake up the coordinator
                        state = CoordinatorState::Syncing {
                            sync_head_number: Arc::new(Mutex::new(0)),
                            start_block_number: from_block_number,
                            downloaders: initial_downloaders,
                            pending_header_downloads: VecDeque::new(),
                            downloaded_headers: Vec::new(),
                            kademlia,
                        };

                        state.get_sync_head_block_number(to_block_head).await;

                        let _ = handle.clone().cast(Self::CastMsg::DownloadHeaders).await;
                    }
                }

                CastResponse::NoReply(state)
            }
            Self::CastMsg::DownloadHeaders => {
                METRICS
                    .headers_download_start_time
                    .lock()
                    .await
                    .replace(SystemTime::now());

                state.prepare_download_tasks().await;

                let _ = handle
                    .clone()
                    .cast(Self::CastMsg::AssignDownloadTasks)
                    .await
                    .inspect_err(|err| {
                        error!("Failed to self cast AssignTasks after preparing tasks to download headers: {err}");
                    });

                // let _ = handle
                //     .clone()
                //     .cast(Self::CastMsg::RefreshDownloaders)
                //     .await
                //     .inspect_err(|err| {
                //         error!("Failed to self cast AssignTasks after preparing tasks to download headers: {err}");
                //     });

                let _ = handle
                    .clone()
                    .cast(Self::CastMsg::UpdateMetrics)
                    .await
                    .inspect_err(|err| {
                        error!("Failed to self cast AssignTasks after preparing tasks to download headers: {err}");
                    });

                CastResponse::NoReply(state)
            }
            Self::CastMsg::DownloadStateTries => {
                state.handle_state_tries_download().await;

                CastResponse::NoReply(state)
            }
            Self::CastMsg::DownloadStorageTries => {
                state.handle_storage_tries_download().await;

                CastResponse::NoReply(state)
            }
            Self::CastMsg::HeadersDownloaded {
                peer_id,
                downloaded_headers,
                assigned_start_block,
                assigned_chunk_limit,
                download_error,
            } => {
                let _ = state
                    .handle_headers_downloaded(
                        peer_id,
                        downloaded_headers,
                        assigned_start_block,
                        assigned_chunk_limit,
                        download_error,
                    )
                    .await
                    .inspect_err(|err| {
                        error!("Failed to handle downloaded headers: {err}");
                    });

                let _ = handle
                    .clone()
                    .cast(Self::CastMsg::AssignDownloadTasks)
                    .await;

                CastResponse::NoReply(state)
            }
            Self::CastMsg::AssignDownloadTasks => {
                state.handle_assign_download_tasks(handle.clone()).await;

                // send_after(
                //     Duration::from_millis(0),
                //     handle.clone(),
                //     Self::CastMsg::AssignDownloadTasks,
                // );

                CastResponse::NoReply(state)
            }
            // Self::CastMsg::RefreshDownloaders => {
            //     state.handle_refresh_downloaders().await;

            //     send_after(
            //         Duration::from_secs(0),
            //         handle.clone(),
            //         Self::CastMsg::RefreshDownloaders,
            //     );

            //     CastResponse::NoReply(state)
            // }
            Self::CastMsg::UpdateMetrics => {
                state.handle_update_metrics().await;

                send_after(
                    Duration::from_secs(1),
                    handle.clone(),
                    Self::CastMsg::UpdateMetrics,
                );

                CastResponse::NoReply(state)
            }
        }
    }
}
