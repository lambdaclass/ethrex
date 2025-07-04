use std::{collections::BTreeMap, sync::Arc, time::Duration};

use bytes::Bytes;
use ethrex_common::{
    H256, U256,
    types::{AccountState, Block, BlockBody, BlockHeader, Receipt, validate_block_body},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::Nibbles;
use ethrex_trie::{Node, verify_range};
use tokio::sync::Mutex;

use crate::{
    kademlia::{KademliaTable, PeerChannels, PeerData},
    rlpx::{
        connection::server::CastMessage,
        eth::{
            blocks::{
                BLOCK_HEADER_LIMIT, BlockBodies, BlockHeaders, GetBlockBodies, GetBlockHeaders,
            },
            receipts::GetReceipts,
        },
        message::Message as RLPxMessage,
        p2p::{Capability, SUPPORTED_ETH_CAPABILITIES, SUPPORTED_SNAP_CAPABILITIES},
        snap::{
            AccountRange, ByteCodes, GetAccountRange, GetByteCodes, GetStorageRanges, GetTrieNodes,
            StorageRanges, TrieNodes,
        },
    },
    snap::encodable_to_proof,
};
use tracing::{debug, info, warn};
pub const PEER_REPLY_TIMEOUT: Duration = Duration::from_secs(15);
pub const PEER_SELECT_RETRY_ATTEMPTS: usize = 3;
pub const REQUEST_RETRY_ATTEMPTS: usize = 5;
pub const MAX_RESPONSE_BYTES: u64 = 512 * 1024;
pub const HASH_MAX: H256 = H256([0xFF; 32]);

// Ask as much as 128 block bodies per request
// this magic number is not part of the protocol and is taken from geth, see:
// https://github.com/ethereum/go-ethereum/blob/2585776aabbd4ae9b00050403b42afb0cee968ec/eth/downloader/downloader.go#L42-L43
//
// Note: We noticed that while bigger values are supported
// increasing them may be the cause of peers disconnection
pub const MAX_BLOCK_BODIES_TO_REQUEST: usize = 128;

/// An abstraction over the [KademliaTable] containing logic to make requests to peers
#[derive(Debug, Clone)]
pub struct PeerHandler {
    peer_table: Arc<Mutex<KademliaTable>>,
}

pub enum BlockRequestOrder {
    OldToNew,
    NewToOld,
}

impl PeerHandler {
    pub fn new(peer_table: Arc<Mutex<KademliaTable>>) -> PeerHandler {
        Self { peer_table }
    }

    /// Creates a dummy PeerHandler for tests where interacting with peers is not needed
    /// This should only be used in tests as it won't be able to interact with the node's connected peers
    pub fn dummy() -> PeerHandler {
        let dummy_peer_table = Arc::new(Mutex::new(KademliaTable::new(Default::default())));
        PeerHandler::new(dummy_peer_table)
    }

    /// Helper method to record successful peer response
    async fn record_peer_success(&self, peer_id: H256) {
        if let Ok(mut table) = self.peer_table.try_lock() {
            table.reward_peer(peer_id);
        }
    }

    /// Helper method to record failed peer response
    async fn record_peer_failure(&self, peer_id: H256) {
        if let Ok(mut table) = self.peer_table.try_lock() {
            table.penalize_peer(peer_id);
        }
    }

    /// Helper method to record critical peer failure
    /// This is used when the peer returns invalid data or is otherwise unreliable
    async fn record_peer_critical_failure(&self, peer_id: H256) {
        if let Ok(mut table) = self.peer_table.try_lock() {
            table.critically_penalize_peer(peer_id);
        }
    }

    /// Returns the node id and the channel ends to an active peer connection that supports the given capability
    /// The peer is selected randomly, and doesn't guarantee that the selected peer is not currently busy
    /// If no peer is found, this method will try again after 10 seconds
    async fn get_peer_channel_with_retry(
        &self,
        capabilities: &[Capability],
    ) -> Option<(H256, PeerChannels)> {
        for _ in 0..PEER_SELECT_RETRY_ATTEMPTS {
            let table = self.peer_table.lock().await;
            if let Some((id, channels)) = table.get_peer_channels(capabilities) {
                return Some((id, channels));
            };
            // drop the lock early to no block the rest of processes
            drop(table);
            info!("[Sync] No peers available, retrying in 10 sec");
            // This is the unlikely case where we just started the node and don't have peers, wait a bit and try again
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        }
        None
    }

    /// Requests block headers from any suitable peer, starting from the `start` block hash towards either older or newer blocks depending on the order
    /// Returns the block headers or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_block_headers(
        &self,
        start: H256,
        order: BlockRequestOrder,
    ) -> Option<Vec<BlockHeader>> {
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let request_id = rand::random();
            let request = RLPxMessage::GetBlockHeaders(GetBlockHeaders {
                id: request_id,
                startblock: start.into(),
                limit: BLOCK_HEADER_LIMIT,
                skip: 0,
                reverse: matches!(order, BlockRequestOrder::NewToOld),
            });
            let (peer_id, mut peer_channel) = self
                .get_peer_channel_with_retry(&SUPPORTED_ETH_CAPABILITIES)
                .await?;
            let mut receiver = peer_channel.receiver.lock().await;
            if let Err(err) = peer_channel
                .connection
                .cast(CastMessage::BackendMessage(request))
                .await
            {
                self.record_peer_failure(peer_id).await;
                debug!("Failed to send message to peer: {err:?}");
                continue;
            }
            if let Some(block_headers) = tokio::time::timeout(PEER_REPLY_TIMEOUT, async move {
                loop {
                    match receiver.recv().await {
                        Some(RLPxMessage::BlockHeaders(BlockHeaders { id, block_headers }))
                            if id == request_id =>
                        {
                            return Some(block_headers);
                        }
                        // Ignore replies that don't match the expected id (such as late responses)
                        Some(_) => continue,
                        None => return None, // Retry request
                    }
                }
            })
            .await
            .ok()
            .flatten()
            .and_then(|headers| (!headers.is_empty()).then_some(headers))
            {
                if are_block_headers_chained(&block_headers, &order) {
                    self.record_peer_success(peer_id).await;
                    return Some(block_headers);
                } else {
                    warn!(
                        "[SYNCING] Received invalid headers from peer, penalizing peer {peer_id}"
                    );
                    self.record_peer_critical_failure(peer_id).await;
                }
            }
            warn!("[SYNCING] Didn't receive block headers from peer, penalizing peer {peer_id}...");
            self.record_peer_failure(peer_id).await;
        }
        None
    }

    /// Internal method to request block bodies from any suitable peer given their block hashes
    /// Returns the block bodies or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - The requested peer did not return a valid response in the given time limit
    async fn request_block_bodies_inner(
        &self,
        block_hashes: Vec<H256>,
    ) -> Option<(Vec<BlockBody>, H256)> {
        let block_hashes_len = block_hashes.len();
        let request_id = rand::random();
        let request = RLPxMessage::GetBlockBodies(GetBlockBodies {
            id: request_id,
            block_hashes: block_hashes.clone(),
        });
        let (peer_id, mut peer_channel) = self
            .get_peer_channel_with_retry(&SUPPORTED_ETH_CAPABILITIES)
            .await?;
        let mut receiver = peer_channel.receiver.lock().await;
        if let Err(err) = peer_channel
            .connection
            .cast(CastMessage::BackendMessage(request))
            .await
        {
            self.record_peer_failure(peer_id).await;
            debug!("Failed to send message to peer: {err:?}");
            return None;
        }
        if let Some(block_bodies) = tokio::time::timeout(PEER_REPLY_TIMEOUT, async move {
            loop {
                match receiver.recv().await {
                    Some(RLPxMessage::BlockBodies(BlockBodies { id, block_bodies }))
                        if id == request_id =>
                    {
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
            (!bodies.is_empty() && bodies.len() <= block_hashes_len).then_some(bodies)
        }) {
            self.record_peer_success(peer_id).await;
            return Some((block_bodies, peer_id));
        }

        warn!("[SYNCING] Didn't receive block bodies from peer, penalizing peer {peer_id}...");
        self.record_peer_failure(peer_id).await;
        None
    }

    /// Requests block bodies from any suitable peer given their block hashes
    /// Returns the block bodies or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_block_bodies(&self, block_hashes: Vec<H256>) -> Option<Vec<BlockBody>> {
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            if let Some((block_bodies, _)) =
                self.request_block_bodies_inner(block_hashes.clone()).await
            {
                return Some(block_bodies);
            }
        }
        None
    }

    /// Requests block bodies from any suitable peer given their block hashes and validates them
    /// Returns the full block or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    /// - The block bodies are invalid given the block headers
    pub async fn request_and_validate_block_bodies<'a>(
        &self,
        block_hashes: &mut Vec<H256>,
        headers_iter: &mut impl Iterator<Item = &BlockHeader>,
    ) -> Option<Vec<Block>> {
        let original_hashes = block_hashes.clone();
        let headers_vec: Vec<&BlockHeader> = headers_iter.collect();

        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            *block_hashes = original_hashes.clone();
            let mut headers_iter = headers_vec.iter().copied();

            let Some((block_bodies, peer_id)) =
                self.request_block_bodies_inner(block_hashes.clone()).await
            else {
                continue; // Retry on empty response
            };

            let mut blocks: Vec<Block> = vec![];
            let block_bodies_len = block_bodies.len();

            // Push blocks
            for (_, body) in block_hashes.drain(..block_bodies_len).zip(block_bodies) {
                let Some(header) = headers_iter.next() else {
                    debug!("[SYNCING] Header not found for the block bodies received, skipping...");
                    break; // Break out of block creation and retry with different peer
                };

                let block = Block::new(header.clone(), body);
                blocks.push(block);
            }

            // Validate blocks
            if let Some(e) = blocks
                .iter()
                .find_map(|block| validate_block_body(block).err())
            {
                warn!(
                    "[SYNCING] Invalid block body error {e}, discarding peer {peer_id} and retrying..."
                );
                self.record_peer_critical_failure(peer_id).await;
                continue; // Retry on validation failure
            }

            return Some(blocks);
        }
        None
    }

    /// Requests all receipts in a set of blocks from any suitable peer given their block hashes
    /// Returns the lists of receipts or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_receipts(&self, block_hashes: Vec<H256>) -> Option<Vec<Vec<Receipt>>> {
        let block_hashes_len = block_hashes.len();
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let request_id = rand::random();
            let request = RLPxMessage::GetReceipts(GetReceipts {
                id: request_id,
                block_hashes: block_hashes.clone(),
            });
            let (_, mut peer_channel) = self
                .get_peer_channel_with_retry(&SUPPORTED_ETH_CAPABILITIES)
                .await?;
            let mut receiver = peer_channel.receiver.lock().await;
            if let Err(err) = peer_channel
                .connection
                .cast(CastMessage::BackendMessage(request))
                .await
            {
                debug!("Failed to send message to peer: {err:?}");
                continue;
            }
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
                return Some(receipts);
            }
        }
        None
    }

    /// Requests an account range from any suitable peer given the state trie's root and the starting hash and the limit hash.
    /// Will also return a boolean indicating if there is more state to be fetched towards the right of the trie
    /// (Note that the boolean will be true even if the remaining state is ouside the boundary set by the limit hash)
    /// Returns the account range or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_account_range(
        &self,
        state_root: H256,
        start: H256,
        limit: H256,
    ) -> Option<(Vec<H256>, Vec<AccountState>, bool)> {
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let request_id = rand::random();
            let request = RLPxMessage::GetAccountRange(GetAccountRange {
                id: request_id,
                root_hash: state_root,
                starting_hash: start,
                limit_hash: limit,
                response_bytes: MAX_RESPONSE_BYTES,
            });
            let (_, mut peer_channel) = self
                .get_peer_channel_with_retry(&SUPPORTED_SNAP_CAPABILITIES)
                .await?;
            let mut receiver = peer_channel.receiver.lock().await;
            if let Err(err) = peer_channel
                .connection
                .cast(CastMessage::BackendMessage(request))
                .await
            {
                debug!("Failed to send message to peer: {err:?}");
                continue;
            }
            if let Some((accounts, proof)) = tokio::time::timeout(PEER_REPLY_TIMEOUT, async move {
                loop {
                    match receiver.recv().await {
                        Some(RLPxMessage::AccountRange(AccountRange {
                            id,
                            accounts,
                            proof,
                        })) if id == request_id => return Some((accounts, proof)),
                        // Ignore replies that don't match the expected id (such as late responses)
                        Some(_) => continue,
                        None => return None,
                    }
                }
            })
            .await
            .ok()
            .flatten()
            {
                // Unzip & validate response
                let proof = encodable_to_proof(&proof);
                let (account_hashes, accounts): (Vec<_>, Vec<_>) = accounts
                    .into_iter()
                    .map(|unit| (unit.hash, AccountState::from(unit.account)))
                    .unzip();
                let encoded_accounts = accounts
                    .iter()
                    .map(|acc| acc.encode_to_vec())
                    .collect::<Vec<_>>();
                if let Ok(should_continue) = verify_range(
                    state_root,
                    &start,
                    &account_hashes,
                    &encoded_accounts,
                    &proof,
                ) {
                    return Some((account_hashes, accounts, should_continue));
                }
            }
        }
        None
    }

    /// Requests bytecodes for the given code hashes
    /// Returns the bytecodes or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_bytecodes(&self, hashes: Vec<H256>) -> Option<Vec<Bytes>> {
        let hashes_len = hashes.len();
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let request_id = rand::random();
            let request = RLPxMessage::GetByteCodes(GetByteCodes {
                id: request_id,
                hashes: hashes.clone(),
                bytes: MAX_RESPONSE_BYTES,
            });
            let (_, mut peer_channel) = self
                .get_peer_channel_with_retry(&SUPPORTED_SNAP_CAPABILITIES)
                .await?;
            let mut receiver = peer_channel.receiver.lock().await;
            if let Err(err) = peer_channel
                .connection
                .cast(CastMessage::BackendMessage(request))
                .await
            {
                debug!("Failed to send message to peer: {err:?}");
                continue;
            }
            if let Some(codes) = tokio::time::timeout(PEER_REPLY_TIMEOUT, async move {
                loop {
                    match receiver.recv().await {
                        Some(RLPxMessage::ByteCodes(ByteCodes { id, codes }))
                            if id == request_id =>
                        {
                            return Some(codes);
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
            .and_then(|codes| (!codes.is_empty() && codes.len() <= hashes_len).then_some(codes))
            {
                return Some(codes);
            }
        }
        None
    }

    /// Requests storage ranges for accounts given their hashed address and storage roots, and the root of their state trie
    /// account_hashes & storage_roots must have the same length
    /// storage_roots must not contain empty trie hashes, we will treat empty ranges as invalid responses
    /// Returns true if the last account's storage was not completely fetched by the request
    /// Returns the list of hashed storage keys and values for each account's storage or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_storage_ranges(
        &self,
        state_root: H256,
        mut storage_roots: Vec<H256>,
        account_hashes: Vec<H256>,
        start: H256,
    ) -> Option<(Vec<Vec<H256>>, Vec<Vec<U256>>, bool)> {
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let request_id = rand::random();
            let request = RLPxMessage::GetStorageRanges(GetStorageRanges {
                id: request_id,
                root_hash: state_root,
                account_hashes: account_hashes.clone(),
                starting_hash: start,
                limit_hash: HASH_MAX,
                response_bytes: MAX_RESPONSE_BYTES,
            });
            let (_, mut peer_channel) = self
                .get_peer_channel_with_retry(&SUPPORTED_SNAP_CAPABILITIES)
                .await?;
            let mut receiver = peer_channel.receiver.lock().await;
            if let Err(err) = peer_channel
                .connection
                .cast(CastMessage::BackendMessage(request))
                .await
            {
                debug!("Failed to send message to peer: {err:?}");
                continue;
            }
            if let Some((mut slots, proof)) = tokio::time::timeout(PEER_REPLY_TIMEOUT, async move {
                loop {
                    match receiver.recv().await {
                        Some(RLPxMessage::StorageRanges(StorageRanges { id, slots, proof }))
                            if id == request_id =>
                        {
                            return Some((slots, proof));
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
            {
                // Check we got a reasonable amount of storage ranges
                if slots.len() > storage_roots.len() || slots.is_empty() {
                    return None;
                }
                // Unzip & validate response
                let proof = encodable_to_proof(&proof);
                let mut storage_keys = vec![];
                let mut storage_values = vec![];
                let mut should_continue = false;
                // Validate each storage range
                while !slots.is_empty() {
                    let (hashed_keys, values): (Vec<_>, Vec<_>) = slots
                        .remove(0)
                        .into_iter()
                        .map(|slot| (slot.hash, slot.data))
                        .unzip();
                    // We won't accept empty storage ranges
                    if hashed_keys.is_empty() {
                        continue;
                    }
                    let encoded_values = values
                        .iter()
                        .map(|val| val.encode_to_vec())
                        .collect::<Vec<_>>();
                    let storage_root = storage_roots.remove(0);

                    // The proof corresponds to the last slot, for the previous ones the slot must be the full range without edge proofs
                    if slots.is_empty() && !proof.is_empty() {
                        let Ok(sc) = verify_range(
                            storage_root,
                            &start,
                            &hashed_keys,
                            &encoded_values,
                            &proof,
                        ) else {
                            continue;
                        };
                        should_continue = sc;
                    } else if verify_range(storage_root, &start, &hashed_keys, &encoded_values, &[])
                        .is_err()
                    {
                        continue;
                    }

                    storage_keys.push(hashed_keys);
                    storage_values.push(values);
                }
                return Some((storage_keys, storage_values, should_continue));
            }
        }
        None
    }

    /// Requests state trie nodes given the root of the trie where they are contained and their path (be them full or partial)
    /// Returns the nodes or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_state_trienodes(
        &self,
        state_root: H256,
        paths: Vec<Nibbles>,
    ) -> Option<Vec<Node>> {
        let expected_nodes = paths.len();
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let request_id = rand::random();
            let request = RLPxMessage::GetTrieNodes(GetTrieNodes {
                id: request_id,
                root_hash: state_root,
                // [acc_path, acc_path,...] -> [[acc_path], [acc_path]]
                paths: paths
                    .iter()
                    .map(|vec| vec![Bytes::from(vec.encode_compact())])
                    .collect(),
                bytes: MAX_RESPONSE_BYTES,
            });
            let (_, mut peer_channel) = self
                .get_peer_channel_with_retry(&SUPPORTED_SNAP_CAPABILITIES)
                .await?;
            let mut receiver = peer_channel.receiver.lock().await;
            if let Err(err) = peer_channel
                .connection
                .cast(CastMessage::BackendMessage(request))
                .await
            {
                debug!("Failed to send message to peer: {err:?}");
                continue;
            }
            if let Some(nodes) = tokio::time::timeout(PEER_REPLY_TIMEOUT, async move {
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
                return Some(nodes);
            }
        }
        None
    }

    /// Requests storage trie nodes given the root of the state trie where they are contained and
    /// a hashmap mapping the path to the account in the state trie (aka hashed address) to the paths to the nodes in its storage trie (can be full or partial)
    /// Returns the nodes or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_storage_trienodes(
        &self,
        state_root: H256,
        paths: BTreeMap<H256, Vec<Nibbles>>,
    ) -> Option<Vec<Node>> {
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let request_id = rand::random();
            let expected_nodes = paths.iter().fold(0, |acc, item| acc + item.1.len());
            let request = RLPxMessage::GetTrieNodes(GetTrieNodes {
                id: request_id,
                root_hash: state_root,
                // {acc_path: [path, path, ...]} -> [[acc_path, path, path, ...]]
                paths: paths
                    .iter()
                    .map(|(acc_path, paths)| {
                        [
                            vec![Bytes::from(acc_path.0.to_vec())],
                            paths
                                .iter()
                                .map(|path| Bytes::from(path.encode_compact()))
                                .collect(),
                        ]
                        .concat()
                    })
                    .collect(),
                bytes: MAX_RESPONSE_BYTES,
            });
            let (_, mut peer_channel) = self
                .get_peer_channel_with_retry(&SUPPORTED_SNAP_CAPABILITIES)
                .await?;
            let mut receiver = peer_channel.receiver.lock().await;
            if let Err(err) = peer_channel
                .connection
                .cast(CastMessage::BackendMessage(request))
                .await
            {
                debug!("Failed to send message to peer: {err:?}");
                continue;
            }
            if let Some(nodes) = tokio::time::timeout(PEER_REPLY_TIMEOUT, async move {
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
                return Some(nodes);
            }
        }
        None
    }

    /// Requests a single storage range for an accouns given its hashed address and storage root, and the root of its state trie
    /// This is a simplified version of `request_storage_range` meant to be used for large tries that require their own single requests
    /// account_hashes & storage_roots must have the same length
    /// storage_root must not be an empty trie hash, we will treat empty ranges as invalid responses
    /// Returns true if the account's storage was not completely fetched by the request
    /// Returns the list of hashed storage keys and values for the account's storage or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_storage_range(
        &self,
        state_root: H256,
        storage_root: H256,
        account_hash: H256,
        start: H256,
    ) -> Option<(Vec<H256>, Vec<U256>, bool)> {
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let request_id = rand::random();
            let request = RLPxMessage::GetStorageRanges(GetStorageRanges {
                id: request_id,
                root_hash: state_root,
                account_hashes: vec![account_hash],
                starting_hash: start,
                limit_hash: HASH_MAX,
                response_bytes: MAX_RESPONSE_BYTES,
            });
            let (_, mut peer_channel) = self
                .get_peer_channel_with_retry(&SUPPORTED_SNAP_CAPABILITIES)
                .await?;
            let mut receiver = peer_channel.receiver.lock().await;
            if let Err(err) = peer_channel
                .connection
                .cast(CastMessage::BackendMessage(request))
                .await
            {
                debug!("Failed to send message to peer: {err:?}");
                continue;
            }
            if let Some((mut slots, proof)) = tokio::time::timeout(PEER_REPLY_TIMEOUT, async move {
                loop {
                    match receiver.recv().await {
                        Some(RLPxMessage::StorageRanges(StorageRanges { id, slots, proof }))
                            if id == request_id =>
                        {
                            return Some((slots, proof));
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
            {
                // Check we got a reasonable amount of storage ranges
                if slots.len() != 1 {
                    return None;
                }
                // Unzip & validate response
                let proof = encodable_to_proof(&proof);
                let (storage_keys, storage_values): (Vec<H256>, Vec<U256>) = slots
                    .remove(0)
                    .into_iter()
                    .map(|slot| (slot.hash, slot.data))
                    .unzip();
                let encoded_values = storage_values
                    .iter()
                    .map(|val| val.encode_to_vec())
                    .collect::<Vec<_>>();
                // Verify storage range
                if let Ok(should_continue) =
                    verify_range(storage_root, &start, &storage_keys, &encoded_values, &proof)
                {
                    return Some((storage_keys, storage_values, should_continue));
                }
            }
        }
        None
    }

    /// Returns the PeerData for each connected Peer
    /// Returns None if it fails to aquire the lock on the kademlia table
    pub fn read_connected_peers(&self) -> Option<Vec<PeerData>> {
        Some(
            self.peer_table
                .try_lock()
                .ok()?
                .filter_peers(&|peer| peer.is_connected)
                .cloned()
                .collect::<Vec<_>>(),
        )
    }

    pub async fn count_total_peers(&self) -> usize {
        self.peer_table.lock().await.iter_peers().count()
    }

    pub async fn remove_peer(&self, peer_id: H256) {
        debug!("Removing peer with id {:?}", peer_id);
        let mut table = self.peer_table.lock().await;
        table.replace_peer(peer_id);
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
