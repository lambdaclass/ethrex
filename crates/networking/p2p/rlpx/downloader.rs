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
    peer_handler::{BlockRequestOrder, BytecodeTaskResult, HASH_MAX, StorageTaskResult},
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
        task_sender: Sender<BytecodeTaskResult>,
        hashes_to_request: Vec<H256>,
        chunk_start: usize,
        chunk_end: usize,
    },
    StorageRanges {
        task_sender: Sender<StorageTaskResult>,
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
            DownloaderCallRequest::Receipts { block_hashes } => {
                let block_hashes_len = block_hashes.len();

                let request_id = rand::random();
                let request = RLPxMessage::GetReceipts(GetReceipts {
                    id: request_id,
                    block_hashes,
                });

                if let Err(err) = self
                    .peer_channels
                    .connection
                    .cast(CastMessage::BackendMessage(request))
                    .await
                {
                    debug!("Failed to send message to peer: {err:?}");
                    return CallResponse::Stop(DownloaderCallResponse::Receipts(None));
                }

                let mut receiver = self.peer_channels.receiver.lock().await;
                if let Some(receipts) = tokio::time::timeout(RECEIPTS_REPLY_TIMEOUT, async move {
                    loop {
                        match receiver.recv().await {
                            Some(RLPxMessage::Receipts(receipts)) => {
                                if receipts.get_id() == request_id {
                                    return Some(receipts.get_receipts());
                                }
                                return None;
                            }
                            // Ignore replies that don't match the expected id (such as late responses)
                            Some(_) => continue,
                            None => return None,
                        }
                    }
                })
                .await
                .ok()
                .flatten()
                .and_then(|receipts|
                    // Check that the response is not empty and does not contain more bodies than the ones requested
                    (!receipts.is_empty() && receipts.len() <= block_hashes_len).then_some(receipts))
                {
                    return CallResponse::Stop(DownloaderCallResponse::Receipts(
                        Some(receipts),
                    ));
                }

                return CallResponse::Stop(DownloaderCallResponse::Receipts(None));
            }
            DownloaderCallRequest::BlockBodies { block_hashes } => {
                let request_id = rand::random();
                let request = RLPxMessage::GetBlockBodies(GetBlockBodies {
                    id: request_id,
                    block_hashes: block_hashes.clone(),
                });
                let mut receiver = self.peer_channels.receiver.lock().await;
                if let Err(err) = self
                    .peer_channels
                    .connection
                    .cast(CastMessage::BackendMessage(request))
                    .await
                {
                    debug!("Failed to send message to peer: {err:?}");
                    return CallResponse::Stop(DownloaderCallResponse::NotFound);
                }

                if let Some(block_bodies) =
                    tokio::time::timeout(BLOCK_BODIES_REPLY_TIMEOUT, async move {
                        loop {
                            match receiver.recv().await {
                                Some(RLPxMessage::BlockBodies(BlockBodies {
                                    id,
                                    block_bodies,
                                })) if id == request_id => {
                                    return Some(block_bodies);
                                }
                                // Ignore replies that don't match the expected id (such as late responses)
                                Some(_) => continue,
                                None => return None,
                            }
                        }
                    })
                    .await
                    .ok()
                    .flatten()
                    .and_then(|bodies| {
                        // Check that the response is not empty and does not contain more bodies than the ones requested
                        (!bodies.is_empty() && bodies.len() <= block_hashes.len()).then_some(bodies)
                    })
                {
                    return CallResponse::Stop(DownloaderCallResponse::BlockBodies(block_bodies));
                }

                return CallResponse::Stop(DownloaderCallResponse::NotFound);
            }
            DownloaderCallRequest::TrieNodes { root_hash, paths } => {
                let request_id = rand::random();
                let expected_nodes = paths.len();
                let request = RLPxMessage::GetTrieNodes(GetTrieNodes {
                    id: request_id,
                    root_hash,
                    // [acc_path, acc_path,...] -> [[acc_path], [acc_path]]
                    paths,
                    bytes: MAX_RESPONSE_BYTES,
                });
                let mut receiver = self.peer_channels.receiver.lock().await;
                if let Err(err) = self
                    .peer_channels
                    .connection
                    .cast(CastMessage::BackendMessage(request))
                    .await
                {
                    debug!("Failed to send message to peer: {err:?}");
                    return CallResponse::Stop(DownloaderCallResponse::NotFound);
                }
                if let Some(nodes) = tokio::time::timeout(RECEIPTS_REPLY_TIMEOUT, async move {
                    loop {
                        match receiver.recv().await {
                            Some(RLPxMessage::TrieNodes(TrieNodes { id, nodes }))
                                if id == request_id =>
                            {
                                return Some(nodes);
                            }
                            // Ignore replies that don't match the expected id (such as late responses)
                            Some(_) => continue,
                            None => return None,
                        }
                    }
                })
                .await
                .ok()
                .flatten()
                .and_then(|nodes| {
                    (!nodes.is_empty() && nodes.len() <= expected_nodes)
                        .then(|| {
                            nodes
                                .iter()
                                .map(|node| Node::decode_raw(node))
                                .collect::<Result<Vec<_>, _>>()
                                .ok()
                        })
                        .flatten()
                }) {
                    return CallResponse::Stop(DownloaderCallResponse::TrieNodes(nodes));
                }
                return CallResponse::Stop(DownloaderCallResponse::NotFound);
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
            DownloaderCastRequest::AccountRange {
                task_sender,
                root_hash,
                starting_hash,
                limit_hash,
            } => {
                let request_id = rand::random();
                let request = RLPxMessage::GetAccountRange(GetAccountRange {
                    id: request_id,
                    root_hash,
                    starting_hash,
                    limit_hash,
                    response_bytes: MAX_RESPONSE_BYTES,
                });

                let mut receiver = self.peer_channels.receiver.lock().await;
                if let Err(err) = (&mut self.peer_channels.connection)
                    .cast(CastMessage::BackendMessage(request))
                    .await
                {
                    error!("Failed to send message to peer: {err:?}");
                    let msg = (Vec::new(), self.peer_id, Some((starting_hash, limit_hash)));
                    self.send_through_response_channel(task_sender, msg).await;
                    return CastResponse::Stop;
                }
                if let Some((accounts, proof)) =
                    tokio::time::timeout(BYTECODE_REPLY_TIMEOUT, async move {
                        loop {
                            match receiver.recv().await {
                                Some(RLPxMessage::AccountRange(AccountRange {
                                    id,
                                    accounts,
                                    proof,
                                })) if id == request_id => return Some((accounts, proof)),
                                Some(_) => continue,
                                None => return None,
                            }
                        }
                    })
                    .await
                    .ok()
                    .flatten()
                {
                    if accounts.is_empty() {
                        let msg = (Vec::new(), self.peer_id, Some((starting_hash, limit_hash)));
                        self.send_through_response_channel(task_sender, msg).await;
                        return CastResponse::Stop;
                    }
                    // Unzip & validate response
                    let proof = encodable_to_proof(&proof);
                    let (account_hashes, account_states): (Vec<_>, Vec<_>) = accounts
                        .clone()
                        .into_iter()
                        .map(|unit| (unit.hash, AccountState::from(unit.account)))
                        .unzip();
                    let encoded_accounts = account_states
                        .iter()
                        .map(|acc| acc.encode_to_vec())
                        .collect::<Vec<_>>();

                    let Ok(should_continue) = verify_range(
                        root_hash,
                        &starting_hash,
                        &account_hashes,
                        &encoded_accounts,
                        &proof,
                    ) else {
                        let msg = (Vec::new(), self.peer_id, Some((starting_hash, limit_hash)));
                        self.send_through_response_channel(task_sender, msg).await;
                        return CastResponse::Stop;
                    };

                    // If the range has more accounts to fetch, we send the new chunk
                    let chunk_left = if should_continue {
                        let last_hash = account_hashes
                            .last()
                            .expect("we already checked this isn't empty");
                        let new_start_u256 = U256::from_big_endian(&last_hash.0) + 1;
                        let new_start = H256::from_uint(&new_start_u256);
                        Some((new_start, limit_hash))
                    } else {
                        None
                    };

                    let accounts = accounts
                        .into_iter()
                        .filter(|unit| unit.hash <= limit_hash)
                        .collect();
                    let msg = (accounts, self.peer_id, chunk_left);
                    self.send_through_response_channel(task_sender, msg).await;
                } else {
                    tracing::debug!("Failed to get account range");
                    let msg = (Vec::new(), self.peer_id, Some((starting_hash, limit_hash)));
                    self.send_through_response_channel(task_sender, msg).await;
                }
                return CastResponse::Stop;
            }
            DownloaderCastRequest::ByteCode {
                task_sender,
                hashes_to_request,
                chunk_start,
                chunk_end,
            } => {
                let empty_task_result = BytecodeTaskResult {
                    start_index: chunk_start,
                    bytecodes: vec![],
                    peer_id: self.peer_id,
                    remaining_start: chunk_start,
                    remaining_end: chunk_end,
                };
                debug!(
                    "Requesting bytecode from peer {}, chunk: {chunk_start:?} - {chunk_end:?}",
                    self.peer_id
                );
                let request_id = rand::random();
                let request = RLPxMessage::GetByteCodes(GetByteCodes {
                    id: request_id,
                    hashes: hashes_to_request.clone(),
                    bytes: MAX_RESPONSE_BYTES,
                });
                let mut receiver = self.peer_channels.receiver.lock().await;
                if let Err(err) = (self.peer_channels.connection)
                    .cast(CastMessage::BackendMessage(request))
                    .await
                {
                    error!("Failed to send message to peer: {err:?}");
                    self.send_through_response_channel(task_sender, empty_task_result)
                        .await;
                    return CastResponse::Stop;
                }

                if let Some(codes) = tokio::time::timeout(BYTECODE_REPLY_TIMEOUT, async move {
                    loop {
                        match receiver.recv().await {
                            Some(RLPxMessage::ByteCodes(ByteCodes { id, codes }))
                                if id == request_id =>
                            {
                                return Some(codes);
                            }
                            Some(_) => continue,
                            None => return None,
                        }
                    }
                })
                .await
                .ok()
                .flatten()
                {
                    if codes.is_empty() {
                        self.send_through_response_channel(task_sender, empty_task_result)
                            .await;
                        return CastResponse::Stop;
                    }
                    // Validate response by hashing bytecodes
                    let validated_codes: Vec<Bytes> = codes
                        .into_iter()
                        .zip(hashes_to_request)
                        .take_while(|(b, hash)| keccak_hash::keccak(b) == *hash)
                        .map(|(b, _hash)| b)
                        .collect();
                    let msg = BytecodeTaskResult {
                        start_index: chunk_start,
                        remaining_start: chunk_start + validated_codes.len(),
                        bytecodes: validated_codes,
                        peer_id: self.peer_id,
                        remaining_end: chunk_end,
                    };
                    self.send_through_response_channel(task_sender, msg).await;
                } else {
                    tracing::error!("Failed to get bytecode");
                    self.send_through_response_channel(task_sender, empty_task_result)
                        .await;
                }

                CastResponse::Stop
            }
            DownloaderCastRequest::StorageRanges {
                task_sender,
                start_index,
                end_index,
                start_hash,
                end_hash,
                state_root,
                chunk_account_hashes,
                chunk_storage_roots,
            } => {
                let empty_task_result = StorageTaskResult {
                    start_index,
                    account_storages: Vec::new(),
                    peer_id: self.peer_id,
                    remaining_start: start_index,
                    remaining_end: end_index,
                    remaining_hash_range: (start_hash, end_hash),
                };
                let request_id = rand::random();
                let request = RLPxMessage::GetStorageRanges(GetStorageRanges {
                    id: request_id,
                    root_hash: state_root,
                    account_hashes: chunk_account_hashes,
                    starting_hash: start_hash,
                    limit_hash: end_hash.unwrap_or(HASH_MAX),
                    response_bytes: MAX_RESPONSE_BYTES,
                });
                let mut receiver = self.peer_channels.receiver.lock().await;
                if let Err(err) = (self.peer_channels.connection)
                    .cast(CastMessage::BackendMessage(request))
                    .await
                {
                    error!("Failed to send message to peer: {err:?}");
                    self.send_through_response_channel(task_sender, empty_task_result)
                        .await;
                    return CastResponse::Stop;
                }
                let request_result =
                    tokio::time::timeout(STORAGE_RANGE_REPLY_TIMEOUT, async move {
                        loop {
                            match receiver.recv().await {
                                Some(RLPxMessage::StorageRanges(StorageRanges {
                                    id,
                                    slots,
                                    proof,
                                })) if id == request_id => return Some((slots, proof)),
                                Some(_) => continue,
                                None => return None,
                            }
                        }
                    })
                    .await
                    .ok()
                    .flatten();
                let Some((slots, proof)) = request_result else {
                    tracing::debug!("Failed to get storage range");
                    self.send_through_response_channel(task_sender, empty_task_result)
                        .await;
                    return CastResponse::Stop;
                };
                if slots.is_empty() && proof.is_empty() {
                    tracing::debug!("Received empty account range");
                    self.send_through_response_channel(task_sender, empty_task_result)
                        .await;
                    return CastResponse::Stop;
                }
                // Check we got some data and no more than the requested amount
                if slots.len() > chunk_storage_roots.len() || slots.is_empty() {
                    self.send_through_response_channel(task_sender, empty_task_result)
                        .await;
                    return CastResponse::Stop;
                }
                // Unzip & validate response
                let proof = encodable_to_proof(&proof);
                let mut account_storages: Vec<Vec<(H256, U256)>> = vec![];
                let mut should_continue = false;
                // Validate each storage range
                let mut storage_roots = chunk_storage_roots.into_iter();
                let last_slot_index = slots.len() - 1;
                for (i, next_account_slots) in slots.into_iter().enumerate() {
                    // We won't accept empty storage ranges
                    if next_account_slots.is_empty() {
                        // This shouldn't happen
                        error!("Received empty storage range, skipping");
                        self.send_through_response_channel(task_sender, empty_task_result)
                            .await;
                        return CastResponse::Stop;
                    }
                    let encoded_values = next_account_slots
                        .iter()
                        .map(|slot| slot.data.encode_to_vec())
                        .collect::<Vec<_>>();
                    let hashed_keys: Vec<_> =
                        next_account_slots.iter().map(|slot| slot.hash).collect();

                    let storage_root = match storage_roots.next() {
                        Some(root) => root,
                        None => {
                            error!("No storage root for account {i}");
                            self.send_through_response_channel(task_sender, empty_task_result)
                                .await;
                            return CastResponse::Stop;
                        }
                    };

                    // The proof corresponds to the last slot, for the previous ones the slot must be the full range without edge proofs
                    if i == last_slot_index && !proof.is_empty() {
                        let Ok(sc) = verify_range(
                            storage_root,
                            &start_hash,
                            &hashed_keys,
                            &encoded_values,
                            &proof,
                        ) else {
                            self.send_through_response_channel(task_sender, empty_task_result)
                                .await;
                            return CastResponse::Stop;
                        };
                        should_continue = sc;
                    } else if verify_range(
                        storage_root,
                        &start_hash,
                        &hashed_keys,
                        &encoded_values,
                        &[],
                    )
                    .is_err()
                    {
                        self.send_through_response_channel(task_sender, empty_task_result)
                            .await;
                        return CastResponse::Stop;
                    }

                    account_storages.push(
                        next_account_slots
                            .iter()
                            .map(|slot| (slot.hash, slot.data))
                            .collect(),
                    );
                }
                let (remaining_start, remaining_end, remaining_start_hash) = if should_continue {
                    let last_account_storage = match account_storages.last() {
                        Some(storage) => storage,
                        None => {
                            self.send_through_response_channel(task_sender, empty_task_result)
                                .await;
                            error!("No account storage found, this shouldn't happen");
                            return CastResponse::Stop;
                        }
                    };
                    let (last_hash, _) = match last_account_storage.last() {
                        Some(last_hash) => last_hash,
                        None => {
                            self.send_through_response_channel(task_sender, empty_task_result)
                                .await;
                            error!("No last hash found, this shouldn't happen");
                            return CastResponse::Stop;
                        }
                    };
                    let next_hash_u256 =
                        U256::from_big_endian(&last_hash.0).saturating_add(1.into());
                    let next_hash = H256::from_uint(&next_hash_u256);
                    (
                        start_index + account_storages.len() - 1,
                        end_index,
                        next_hash,
                    )
                } else {
                    (
                        start_index + account_storages.len(),
                        end_index,
                        H256::zero(),
                    )
                };
                let task_result = StorageTaskResult {
                    start_index,
                    account_storages,
                    peer_id: self.peer_id,
                    remaining_start,
                    remaining_end,
                    remaining_hash_range: (remaining_start_hash, end_hash),
                };
                self.send_through_response_channel(task_sender, task_result)
                    .await;

                CastResponse::NoReply
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
