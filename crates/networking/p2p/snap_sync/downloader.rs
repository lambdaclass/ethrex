use std::time::Duration;

use ethrex_common::{
    BigEndianHash, U256,
    types::{AccountState, BlockHeader},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::verify_range;
use keccak_hash::H256;
use spawned_concurrency::tasks::{CastResponse, GenServer, GenServerHandle, InitResult};
use tokio::sync::mpsc::Sender;
use tracing::{debug, error, warn};

use crate::{
    kademlia::PeerChannels,
    peer_handler::{BlockRequestOrder, MAX_RESPONSE_BYTES},
    rlpx::{
        Message as RLPxMessage,
        connection::server::{CallMessage, CastMessage, OutMessage},
        eth::blocks::{BlockHeaders, GetBlockHeaders, HashOrNumber},
        snap::{AccountRange, AccountRangeUnit, GetAccountRange},
    },
    snap::encodable_to_proof,
};

#[derive(Clone)]
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
pub enum DownloaderRequest {
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
}

impl GenServer for Downloader {
    type Error = ();
    type CallMsg = ();
    type CastMsg = DownloaderRequest;
    type OutMsg = ();

    async fn init(
        self,
        _handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
    ) -> Result<InitResult<Self>, Self::Error> {
        Ok(InitResult::Success(self))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            DownloaderRequest::Headers {
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
                self.peer_channels
                    .connection
                    .cast(CastMessage::BackendMessage(request))
                    .await
                    .map_err(|e| format!("Failed to send message to peer {}: {e}", self.peer_id))
                    .unwrap(); // TODO: handle unwrap

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
                        error!(
                            "Failed to receive block headers from peer: {}",
                            self.peer_id
                        );
                        task_sender
                            .send((vec![], self.peer_id, start_block, chunk_limit))
                            .await
                            .unwrap(); // TODO: handle unwrap
                        return CastResponse::Stop;
                    }
                };

                if are_block_headers_chained(&block_headers, &BlockRequestOrder::OldToNew) {
                    task_sender
                        .send((block_headers, self.peer_id, start_block, chunk_limit))
                        .await
                        .unwrap(); // TODO: handle unwrap
                } else {
                    warn!(
                        "[SYNCING] Received invalid headers from peer: {}",
                        self.peer_id
                    );
                    task_sender
                        .send((vec![], self.peer_id, start_block, chunk_limit))
                        .await
                        .unwrap(); // TODO: handle unwrap
                }

                // Nothing to do after completion, stop actor
                CastResponse::Stop
            }
            DownloaderRequest::AccountRange {
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
                // match &mut self
                //     .peer_channels
                //     .connection
                //     .call(CallMessage::BackendMessage(request))
                //     .await
                // {
                //     Ok(Ok(OutMessage::BackendResponse(RLPxMessage::AccountRange(
                //         AccountRange {
                //             id: _,
                //             accounts,
                //             proof,
                //         },
                //     )))) => {
                //         if accounts.is_empty() {
                //             task_sender
                //                 .send((Vec::new(), self.peer_id, Some((starting_hash, limit_hash))))
                //                 .await
                //                 .ok();
                //             // Too spammy
                //             // tracing::error!("Received empty account range");
                //             // Downloader has done its job, stop it
                //             return CastResponse::Stop;
                //         }
                //         // Unzip & validate response
                //         let proof = encodable_to_proof(&proof);
                //         let (account_hashes, account_states): (Vec<_>, Vec<_>) = accounts
                //             .clone()
                //             .into_iter()
                //             .map(|unit| (unit.hash, AccountState::from(unit.account)))
                //             .unzip();
                //         let encoded_accounts = account_states
                //             .iter()
                //             .map(|acc| acc.encode_to_vec())
                //             .collect::<Vec<_>>();

                //         let Ok(should_continue) = verify_range(
                //             root_hash,
                //             &starting_hash,
                //             &account_hashes,
                //             &encoded_accounts,
                //             &proof,
                //         ) else {
                //             task_sender
                //                 .send((Vec::new(), self.peer_id, Some((starting_hash, limit_hash))))
                //                 .await
                //                 .ok();
                //             tracing::error!("Received invalid account range");
                //             // Downloader has done its job, stop it
                //             return CastResponse::Stop;
                //         };

                //         // If the range has more accounts to fetch, we send the new chunk
                //         let chunk_left = if should_continue {
                //             let last_hash = account_hashes
                //                 .last()
                //                 .expect("we already checked this isn't empty");
                //             let new_start_u256 = U256::from_big_endian(&last_hash.0) + 1;
                //             let new_start = H256::from_uint(&new_start_u256);
                //             Some((new_start, limit_hash))
                //         } else {
                //             None
                //         };
                //         let accounts = accounts.clone();
                //         task_sender
                //             .send((
                //                 accounts
                //                     .into_iter()
                //                     .filter(|unit| unit.hash <= limit_hash)
                //                     .collect(),
                //                 self.peer_id,
                //                 chunk_left,
                //             ))
                //             .await
                //             .ok();
                //     }
                //     _ => {
                //         error!("Failed to send message to peer");
                //         task_sender
                //             .send((Vec::new(), self.peer_id, Some((starting_hash, limit_hash))))
                //             .await
                //             .ok();

                //         // Downloader has done its job, stop it
                //         return CastResponse::Stop;
                //     }
                // }

                // TODO: remove after call implementation
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
