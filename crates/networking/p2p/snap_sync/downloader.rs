use std::time::Duration;

use bytes::Bytes;
use ethrex_common::{
    BigEndianHash, U256,
    types::{AccountState, BlockHeader, Receipt},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::verify_range;
use keccak_hash::H256;
use spawned_concurrency::tasks::{CallResponse, CastResponse, GenServer, GenServerHandle};
use tokio::sync::mpsc::Sender;

use crate::{
    kademlia::PeerChannels,
    peer_handler::{BlockRequestOrder, PEER_REPLY_TIMEOUT, TaskResult},
    rlpx::{
        Message as RLPxMessage,
        connection::server::CastMessage,
        eth::{
            blocks::{BlockHeaders, GetBlockHeaders, HashOrNumber},
            receipts::GetReceipts,
        },
        snap::{AccountRange, AccountRangeUnit, ByteCodes, GetAccountRange, GetByteCodes},
    },
    snap::encodable_to_proof,
};

use crate::peer_handler::MAX_RESPONSE_BYTES; // TODO: move here eventually

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
    Receipts { block_hashes: Vec<H256> },
}

#[derive(Clone)]
pub enum DownloaderCallResponse {
    NotFound,                            // Whatever we were looking for was not found
    CurrentHead(u64),                    // The sync head block number
    Receipts(Option<Vec<Vec<Receipt>>>), // Requested receipts to a given block hash
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
        task_sender: Sender<TaskResult>,
        hashes_to_request: Vec<H256>,
        chunk_start: usize,
        chunk_end: usize,
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
                if let Some(receipts) = tokio::time::timeout(PEER_REPLY_TIMEOUT, async move {
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
            DownloaderCastRequest::AccountRange {
                task_sender,
                root_hash,
                starting_hash,
                limit_hash,
            } => {
                debug!(
                    "Requesting account range from peer {}, chunk: {:?} - {:?}",
                    self.peer_id, starting_hash, limit_hash
                );
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
                    task_sender
                        .send((Vec::new(), self.peer_id, Some((starting_hash, limit_hash))))
                        .await
                        .ok();

                    // Downloader has done its job, stop it
                    return CastResponse::Stop;
                }
                if let Some((accounts, proof)) =
                    tokio::time::timeout(Duration::from_secs(2), async move {
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
                        task_sender
                            .send((Vec::new(), self.peer_id, Some((starting_hash, limit_hash))))
                            .await
                            .ok();
                        // Too spammy
                        // tracing::error!("Received empty account range");
                        // Downloader has done its job, stop it
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
                        task_sender
                            .send((Vec::new(), self.peer_id, Some((starting_hash, limit_hash))))
                            .await
                            .ok();
                        tracing::error!("Received invalid account range");
                        // Downloader has done its job, stop it
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
                    task_sender
                        .send((
                            accounts
                                .into_iter()
                                .filter(|unit| unit.hash <= limit_hash)
                                .collect(),
                            self.peer_id,
                            chunk_left,
                        ))
                        .await
                        .ok();
                } else {
                    tracing::debug!("Failed to get account range");
                    task_sender
                        .send((Vec::new(), self.peer_id, Some((starting_hash, limit_hash))))
                        .await
                        .ok();
                }
                // Downloader has done its job, stop it
                return CastResponse::Stop;
            }
            DownloaderCastRequest::ByteCode {
                task_sender,
                hashes_to_request,
                chunk_start,
                chunk_end,
            } => {
                let empty_task_result = TaskResult {
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
                    task_sender.send(empty_task_result).await.ok();
                    return CastResponse::Stop;
                }

                if let Some(codes) = tokio::time::timeout(Duration::from_secs(2), async move {
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
                        task_sender.send(empty_task_result).await.ok();
                        // Too spammy
                        // tracing::error!("Received empty account range");
                        return CastResponse::Stop;
                    }
                    // Validate response by hashing bytecodes
                    let validated_codes: Vec<Bytes> = codes
                        .into_iter()
                        .zip(hashes_to_request)
                        .take_while(|(b, hash)| keccak_hash::keccak(b) == *hash)
                        .map(|(b, _hash)| b)
                        .collect();
                    let result = TaskResult {
                        start_index: chunk_start,
                        remaining_start: chunk_start + validated_codes.len(),
                        bytecodes: validated_codes,
                        peer_id: self.peer_id,
                        remaining_end: chunk_end,
                    };
                    task_sender.send(result).await.ok();
                } else {
                    tracing::error!("Failed to get bytecode");
                    task_sender.send(empty_task_result).await.ok();
                }

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
