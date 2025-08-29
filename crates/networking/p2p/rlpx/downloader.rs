use std::time::Duration;

use bytes::Bytes;
use ethrex_common::{
    BigEndianHash, U256,
    types::{AccountState, BlockBody, BlockHeader, Receipt},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::{Node, verify_range};
use keccak_hash::H256;
use spawned_concurrency::tasks::{CallResponse, CastResponse, GenServer, GenServerHandle};
use tokio::sync::mpsc::Sender;

use crate::{
    kademlia::PeerChannels,
    peer_handler::{BlockRequestOrder, HASH_MAX},
    rlpx::{
        Message as RLPxMessage,
        connection::server::CastMessage,
        eth::{
            blocks::{BlockBodies, BlockHeaders, GetBlockBodies, GetBlockHeaders, HashOrNumber},
            receipts::GetReceipts,
        },
        snap::{
            AccountRange, AccountRangeUnit, ByteCodes, GetAccountRange, GetByteCodes,
            GetStorageRanges, GetTrieNodes, StorageRanges, TrieNodes,
        },
    },
    snap::encodable_to_proof,
};

pub const MAX_RESPONSE_BYTES: u64 = 512 * 1024;

pub const RECEIPTS_REPLY_TIMEOUT: Duration = Duration::from_secs(15);
pub const BLOCK_BODIES_REPLY_TIMEOUT: Duration = Duration::from_secs(2);
pub const BLOCK_HEADERS_REPLY_TIMEOUT: Duration = Duration::from_secs(2);
pub const BYTECODE_REPLY_TIMEOUT: Duration = Duration::from_secs(4);
pub const ACCOUNT_RANGE_REPLY_TIMEOUT: Duration = Duration::from_secs(2);
pub const STORAGE_RANGE_REPLY_TIMEOUT: Duration = Duration::from_secs(2);
pub const CURRENT_HEAD_REPLY_TIMEOUT: Duration = Duration::from_secs(1);

use tracing::{debug, error, warn};

// channel to send the tasks to the peers
pub struct BytecodeRequestTaskResult {
    pub(crate) start_index: usize,
    pub(crate) bytecodes: Vec<Bytes>,
    pub(crate) peer_id: H256,
    pub(crate) remaining_start: usize,
    pub(crate) remaining_end: usize,
}

#[derive(Clone)]
pub struct StorageRequestTaskResult {
    pub(crate) start_index: usize,
    pub(crate) account_storages: Vec<Vec<(H256, U256)>>,
    pub(crate) peer_id: H256,
    pub(crate) remaining_start: usize,
    pub(crate) remaining_end: usize,
    pub(crate) remaining_hash_range: (H256, Option<H256>),
}

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

    async fn send_through_response_channel<T>(&self, response_channel: Sender<T>, msg: T)
    where
        T: Send + 'static,
    {
        if let Err(_) = response_channel.send(msg).await {
            error!("[SYNCING] Failed to send response though response channel"); // TODO: Irrecoverable?
        }
    }

    pub fn peer_id(&self) -> H256 {
        self.peer_id
    }
}

#[derive(Clone)]
pub enum DownloaderCallRequest {
    // Snap sync calls
    CurrentHead {
        sync_head: H256,
    },
    Receipts {
        block_hashes: Vec<H256>,
    },
    // Full sync calls
    BlockBodies {
        block_hashes: Vec<H256>,
    },
    TrieNodes {
        root_hash: H256,
        paths: Vec<Vec<Bytes>>,
    },
}

#[derive(Clone)]
pub enum DownloaderCallResponse {
    NotFound,                            // Whatever we were looking for was not found
    CurrentHead(u64),                    // The sync head block number
    Receipts(Option<Vec<Vec<Receipt>>>), // Requested receipts to a given block hash
    BlockBodies(Vec<BlockBody>),         // Requested block bodies to given block hashes
    TrieNodes(Vec<Node>),                // Requested trie nodes
}

#[derive(Clone)]
pub enum DownloaderCastRequest {
    Headers {
        task_sender: Sender<(Vec<BlockHeader>, H256, u64, u64)>,
        start_block: u64,
        chunk_limit: u64,
    },
    AccountRange {
        task_sender: Sender<(Vec<AccountRangeUnit>, H256, Option<(H256, H256)>)>,
        root_hash: H256,
        starting_hash: H256,
        limit_hash: H256,
    },
    ByteCode {
        task_sender: Sender<BytecodeRequestTaskResult>,
        hashes_to_request: Vec<H256>,
        chunk_start: usize,
        chunk_end: usize,
    },
    StorageRanges {
        task_sender: Sender<StorageRequestTaskResult>,
        start_index: usize,
        end_index: usize,
        start_hash: H256,
        // end_hash is None if the task is for the first big storage request
        end_hash: Option<H256>,
        state_root: H256,
        chunk_account_hashes: Vec<H256>,
        chunk_storage_roots: Vec<H256>,
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

                if let Err(_) = self
                    .peer_channels
                    .connection
                    .cast(CastMessage::BackendMessage(request.clone()))
                    .await
                {
                    error!("Failed sending backend message to peer");
                    return CallResponse::Stop(DownloaderCallResponse::NotFound);
                }

                let peer_id = self.peer_id;
                match tokio::time::timeout(CURRENT_HEAD_REPLY_TIMEOUT, async move {
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
            _ => todo!(),
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

                if let Err(err) = self
                    .peer_channels
                    .connection
                    .cast(CastMessage::BackendMessage(request))
                    .await
                {
                    warn!("Failed to send message to peer: {err:?}");
                    let msg = (Vec::new(), self.peer_id, start_block, chunk_limit);
                    self.send_through_response_channel(task_sender, msg).await;
                    return CastResponse::Stop;
                };

                let block_headers =
                    match tokio::time::timeout(BLOCK_HEADERS_REPLY_TIMEOUT, async move {
                        loop {
                            match receiver.recv().await {
                                Some(RLPxMessage::BlockHeaders(BlockHeaders {
                                    id,
                                    block_headers,
                                })) if id == request_id => {
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
                            let msg = (Vec::new(), self.peer_id, start_block, chunk_limit);
                            self.send_through_response_channel(task_sender, msg).await;
                            return CastResponse::Stop;
                        }
                    };

                if are_block_headers_chained(&block_headers, &BlockRequestOrder::OldToNew) {
                    let msg = (
                        block_headers.clone(),
                        self.peer_id,
                        start_block,
                        chunk_limit,
                    );
                    self.send_through_response_channel(task_sender, msg).await;
                } else {
                    warn!(
                        "[SYNCING] Received invalid headers from peer: {}",
                        self.peer_id
                    );
                    let msg = (Vec::new(), self.peer_id, start_block, chunk_limit);
                    self.send_through_response_channel(task_sender, msg).await;
                }

                // Nothing to do after completion, stop actor
                CastResponse::Stop
            }
            _ => todo!(),
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
