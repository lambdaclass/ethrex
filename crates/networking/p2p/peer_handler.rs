use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use ethrex_common::{
    types::{AccountState, BlockBody, BlockHeader, Receipt},
    H256, H512, U256,
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::Nibbles;
use ethrex_trie::{verify_range, Node};
use tokio::sync::Mutex;

use crate::{
    kademlia::{KademliaTable, PeerChannels},
    rlpx::{
        eth::{
            blocks::{
                BlockBodies, BlockHeaders, GetBlockBodies, GetBlockHeaders, BLOCK_HEADER_LIMIT,
            },
            receipts::{GetReceipts, Receipts},
        },
        message::Message as RLPxMessage,
        p2p::Capability,
        snap::{
            AccountRange, ByteCodes, GetAccountRange, GetByteCodes, GetStorageRanges, GetTrieNodes,
            StorageRanges, TrieNodes,
        },
    },
    snap::encodable_to_proof,
};
use tracing::{info, warn};
pub const PEER_REPLY_TIMEOUT: Duration = Duration::from_secs(5);
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

    /// Returns the channel ends to an active peer connection that supports the given capability.  
    ///
    /// The peer is selected based on its scoring and if it is not currently busy. If no suitable peer is found, this method retries  
    /// after 10 seconds.
    async fn get_peer_channel_with_retry(
        &self,
        capability: Capability,
    ) -> Option<(PeerChannels, H512)> {
        for _ in 0..PEER_SELECT_RETRY_ATTEMPTS {
            let mut table_lock = self.peer_table.lock().await;
            if let Some(peer) = table_lock.get_idle_peer_with_capability_mut(capability.clone()) {
                peer.set_as_busy();
                return Some((peer.channels.clone().unwrap(), peer.node.node_id));
            };
            // drop the lock early to no block the rest of processes
            drop(table_lock);
            info!("[Sync] No peers available, retrying in 10 sec");
            // This is the unlikely case where we just started the node and don't have peers, wait a bit and try again
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        }
        None
    }

    pub async fn peer_failed_to_respond(&self, node_id: H512) {
        let mut table_lock = self.peer_table.lock().await;
        let Some(peer) = table_lock.get_by_node_id_mut(node_id) else {
            return;
        };

        peer.set_as_idle();
        peer.scoring = peer.scoring.saturating_sub(1);
        if peer.scoring == 0 {
            warn!(
                "Peer {:?} is being replaced. Reason: scoring reached zero.",
                table_lock.replace_peer(node_id)
            )
        }
    }

    pub async fn peer_responded_successfully(&self, node_id: H512, scoring_points: u16) {
        let mut table_lock = self.peer_table.lock().await;
        let Some(peer) = table_lock.get_by_node_id_mut(node_id) else {
            return;
        };

        peer.set_as_idle();
        peer.scoring = peer.scoring.saturating_add(scoring_points);
    }

    /// Sends a request to a peer with the specified capability.
    ///
    /// ### Params
    /// - `request_msg`: A callback function that constructs the request message. This avoids the need to derive `Clone` for `RLPxMessage`,
    ///   as each request attempt takes ownership of the message. This is beneficial because some `RLPxMessage` structures can be expensive to clone.
    /// - `validate_response`: A function that validates the received response. If the response is successful, it is returned.
    ///
    /// The request is retried up to [`REQUEST_RETRY_ATTEMPTS`] times if necessary.
    ///
    /// ### Scoring
    /// Additionally, peers receive points for successful validations and low-latency responses,
    /// so that we prioritize the selection of peers when sending messages.
    async fn send_request<T: crate::rlpx::message::RLPxMessage + 'static>(
        &self,
        cap: Capability,
        request_msg: impl Fn() -> RLPxMessage,
        validate_response: impl Fn(&T) -> bool,
    ) -> Option<T> {
        for _ in 0..REQUEST_RETRY_ATTEMPTS {
            let channels = self.get_peer_channel_with_retry(cap.clone()).await;
            let (channel, node_id) = channels?;

            let mut receiver = channel.receiver.lock().await;
            channel.sender.send(request_msg()).await.ok();
            let since = Instant::now();
            let response = tokio::time::timeout(PEER_REPLY_TIMEOUT, async move {
                match receiver.recv().await {
                    Some(res) => Some(res),
                    None => None,
                }
            })
            .await
            .ok()
            .flatten();

            let Some(response) = response else {
                self.peer_failed_to_respond(node_id).await;
                continue;
            };

            let response: T = response.inner()?;
            let validation = validate_response(&response);
            if !validation {
                self.peer_failed_to_respond(node_id).await;
                return None;
            }

            // Assign 1 point for a successful response.
            // If the response time is under 100ms, assign 1 additional points for speed.
            let mut scoring_points = 1;
            let latency = since.elapsed().as_millis();
            if latency < 100 {
                scoring_points += 1;
            }

            self.peer_responded_successfully(node_id, scoring_points)
                .await;

            return Some(response);
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
        let request_id = rand::random();
        let request = || {
            RLPxMessage::GetBlockHeaders(GetBlockHeaders {
                id: request_id,
                startblock: start.into(),
                limit: BLOCK_HEADER_LIMIT,
                skip: 0,
                reverse: matches!(order, BlockRequestOrder::NewToOld),
            })
        };
        let validation = |response: &BlockHeaders| {
            response.id == request_id && !response.block_headers.is_empty()
        };

        let response = self
            .send_request::<BlockHeaders>(Capability::Eth, request, validation)
            .await?;

        Some(response.block_headers)
    }

    /// Requests block bodies from any suitable peer given their block hashes
    /// Returns the block bodies or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_block_bodies(&self, block_hashes: Vec<H256>) -> Option<Vec<BlockBody>> {
        let block_hashes_len = block_hashes.len();
        let request_id = rand::random();
        let request = || {
            RLPxMessage::GetBlockBodies(GetBlockBodies {
                id: request_id,
                block_hashes: block_hashes.clone(),
            })
        };
        let validation = |response: &BlockBodies| {
            response.id == request_id
                && !response.block_bodies.is_empty()
                && response.block_bodies.len() <= block_hashes_len
        };
        let response = self
            .send_request::<BlockBodies>(Capability::Eth, request, validation)
            .await?;

        Some(response.block_bodies)
    }

    /// Requests all receipts in a set of blocks from any suitable peer given their block hashes
    /// Returns the lists of receipts or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_receipts(&self, block_hashes: Vec<H256>) -> Option<Vec<Vec<Receipt>>> {
        let block_hashes_len = block_hashes.len();
        let request_id = rand::random();
        let request = || {
            RLPxMessage::GetReceipts(GetReceipts {
                id: request_id,
                block_hashes: block_hashes.clone(),
            })
        };
        let validation = |response: &Receipts| {
            response.id == request_id
                && !response.receipts.is_empty()
                && response.receipts.len() <= block_hashes_len
        };
        let response = self
            .send_request::<Receipts>(Capability::Eth, request, validation)
            .await?;

        Some(response.receipts)
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
        let request_id = rand::random();
        let request = || {
            RLPxMessage::GetAccountRange(GetAccountRange {
                id: request_id,
                root_hash: state_root,
                starting_hash: start,
                limit_hash: limit,
                response_bytes: MAX_RESPONSE_BYTES,
            })
        };
        let validation = |response: &AccountRange| response.id == request_id;

        let response = self
            .send_request::<AccountRange>(Capability::Eth, request, validation)
            .await?;

        let (accounts, proof) = (response.accounts, response.proof);

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

        None
    }

    /// Requests bytecodes for the given code hashes
    /// Returns the bytecodes or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_bytecodes(&self, hashes: Vec<H256>) -> Option<Vec<Bytes>> {
        let hashes_len = hashes.len();
        let request_id = rand::random();
        let request = || {
            RLPxMessage::GetByteCodes(GetByteCodes {
                id: request_id,
                hashes: hashes.clone(),
                bytes: MAX_RESPONSE_BYTES,
            })
        };
        let validation = |response: &ByteCodes| {
            response.id == request_id
                && !response.codes.is_empty()
                && response.codes.len() <= hashes_len
        };

        let response = self
            .send_request::<ByteCodes>(Capability::Eth, request, validation)
            .await?;

        Some(response.codes)
    }

    /// Requests storage ranges for accounts given their hashed address and storage roots, and the root of their state trie
    /// account_hashes & storage_roots must have the same length
    /// storage_roots must not contain empty trie hashes, we will treat empty ranges as invalid responses
    /// Returns true if the last account's storage was not completely fetched by the request
    /// Returns the list of hashed storage keys and values for each account's storage or None if:
    /// - There are no available peers (the node just started up or was rejected by all other nodes)
    /// - No peer returned a valid response in the given time and retry limits
    pub async fn request_storage_ranges(
        &mut self,
        state_root: H256,
        mut storage_roots: Vec<H256>,
        account_hashes: Vec<H256>,
        start: H256,
    ) -> Option<(Vec<Vec<H256>>, Vec<Vec<U256>>, bool)> {
        let request_id = rand::random();
        let request = || {
            RLPxMessage::GetStorageRanges(GetStorageRanges {
                id: request_id,
                root_hash: state_root,
                account_hashes: account_hashes.clone(),
                starting_hash: start,
                limit_hash: HASH_MAX,
                response_bytes: MAX_RESPONSE_BYTES,
            })
        };
        let validation = |response: &StorageRanges| {
            // Check we got a reasonable amount of storage ranges
            response.id == request_id
                && !response.slots.is_empty()
                && response.slots.len() <= storage_roots.len()
        };

        let response = self
            .send_request::<StorageRanges>(Capability::Eth, request, validation)
            .await?;

        let (mut slots, proof) = (response.slots, response.proof);

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
                let Ok(sc) =
                    verify_range(storage_root, &start, &hashed_keys, &encoded_values, &proof)
                else {
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

        Some((storage_keys, storage_values, should_continue))
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
        let request_id = rand::random();
        let request = || {
            RLPxMessage::GetTrieNodes(GetTrieNodes {
                id: request_id,
                root_hash: state_root,
                // [acc_path, acc_path,...] -> [[acc_path], [acc_path]]
                paths: paths
                    .iter()
                    .map(|vec| vec![Bytes::from(vec.encode_compact())])
                    .collect(),
                bytes: MAX_RESPONSE_BYTES,
            })
        };

        let validation = |response: &TrieNodes| {
            response.id == request_id
                && !response.nodes.is_empty()
                && response.nodes.len() <= expected_nodes
        };

        let response = self
            .send_request::<TrieNodes>(Capability::Eth, request, validation)
            .await?;

        response
            .nodes
            .iter()
            .map(|node| Node::decode_raw(node))
            .collect::<Result<Vec<_>, _>>()
            .ok()
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
        let request_id = rand::random();
        let expected_nodes = paths.iter().fold(0, |acc, item| acc + item.1.len());
        let request = || {
            RLPxMessage::GetTrieNodes(GetTrieNodes {
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
            })
        };

        let validation = |response: &TrieNodes| {
            response.id == request_id
                && !response.nodes.is_empty()
                && response.nodes.len() <= expected_nodes
        };

        let response = self
            .send_request::<TrieNodes>(Capability::Eth, request, validation)
            .await?;

        response
            .nodes
            .iter()
            .map(|node| Node::decode_raw(node))
            .collect::<Result<Vec<_>, _>>()
            .ok()
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
        let request_id = rand::random();
        let request = || {
            RLPxMessage::GetStorageRanges(GetStorageRanges {
                id: request_id,
                root_hash: state_root,
                account_hashes: vec![account_hash],
                starting_hash: start,
                limit_hash: HASH_MAX,
                response_bytes: MAX_RESPONSE_BYTES,
            })
        };

        let validation =
            |response: &StorageRanges| response.id == request_id && response.slots.len() == 1;

        let response = self
            .send_request::<StorageRanges>(Capability::Eth, request, validation)
            .await?;

        let (mut slots, proof) = (response.slots, response.proof);

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

        None
    }
}
