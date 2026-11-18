//! [`Peers`] over ethrex's devp2p stack: raw snap range requests of width one for proofs,
//! eth requests for headers, bodies and receipts, snap/2 then eth/71 for block access lists,
//! and direct `Transactions` messages for broadcast.
//!
//! Replies are returned as untrusted wire data; [`crate::verify`] checks them. The only
//! checks done here are that a reply answers the request sent and is of the expected kind,
//! which score the peer that sent it.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use ethrex_common::H256;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::{BlockBody, BlockHeader, Receipt, Transaction};
use ethrex_p2p::peer_handler::PeerHandler;
use ethrex_p2p::peer_table::{PeerTable, PeerTableServerProtocol};
use ethrex_p2p::rlpx::Message;
use ethrex_p2p::rlpx::connection::server::PeerConnection;
use ethrex_p2p::rlpx::eth::blocks::{GetBlockHeaders, HashOrNumber};
use ethrex_p2p::rlpx::eth::transactions::Transactions;
use ethrex_p2p::rlpx::p2p::{SUPPORTED_ETH_CAPABILITIES, SUPPORTED_SNAP_CAPABILITIES};
use ethrex_p2p::rlpx::snap::{GetAccountRange, GetByteCodes, GetStorageRanges};
use ethrex_p2p::snap::constants::MAX_RESPONSE_BYTES;
use ethrex_storage::Store;
use tracing::{debug, warn};

use crate::peers::{Census, NetError, PeerKey, Peers, Reply};
use crate::state::Store as PartialStore;
use crate::verify::{AccountRangeRequest, AccountReply, StorageRangeRequest, StorageReply};

/// How long a peer gets to answer a request.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Peers a transaction is sent to.
pub const BROADCAST_PEERS: usize = 8;

/// The devp2p-backed peer set.
#[derive(Clone)]
pub struct P2pPeers {
    peer_table: PeerTable,
    peer_handler: PeerHandler,
    /// The node's own store: headers are written here so the handshake and `BlockRangeUpdate`
    /// announce the verified head.
    store: Store,
}

impl P2pPeers {
    /// Peers over a running p2p stack.
    pub fn new(peer_table: PeerTable, peer_handler: PeerHandler, store: Store) -> Self {
        Self {
            peer_table,
            peer_handler,
            store,
        }
    }

    async fn snap_peer(
        &self,
    ) -> Result<
        (
            PeerKey,
            PeerConnection,
            ethrex_p2p::peer_table::RequestPermit,
        ),
        NetError,
    > {
        self.peer_table
            .get_best_peer(SUPPORTED_SNAP_CAPABILITIES.to_vec())
            .await
            .map_err(|e| NetError::Rejected(e.to_string()))?
            .ok_or(NetError::NoPeer)
    }

    async fn eth_peer(
        &self,
    ) -> Result<
        (
            PeerKey,
            PeerConnection,
            ethrex_p2p::peer_table::RequestPermit,
        ),
        NetError,
    > {
        let selected = self
            .peer_table
            .get_random_peer(SUPPORTED_ETH_CAPABILITIES.to_vec())
            .await
            .map_err(|e| NetError::Rejected(e.to_string()))?
            .ok_or(NetError::NoPeer)?;
        Ok((selected.0, selected.1, selected.2))
    }

    async fn request(
        &self,
        peer: PeerKey,
        connection: &mut PeerConnection,
        message: Message,
    ) -> Result<Message, NetError> {
        match connection.outgoing_request(message, REQUEST_TIMEOUT).await {
            Ok(reply) => {
                let _ = self.peer_table.record_success(peer);
                Ok(reply)
            }
            Err(err) => {
                let _ = self.peer_table.record_failure(peer);
                let text = err.to_string();
                if text.to_ascii_lowercase().contains("timeout") {
                    Err(NetError::Timeout)
                } else {
                    Err(NetError::Rejected(text))
                }
            }
        }
    }

    fn mismatch(&self, peer: PeerKey, what: &str) -> NetError {
        let _ = self.peer_table.record_failure(peer);
        NetError::Rejected(format!("peer answered with {what}"))
    }
}

#[async_trait]
impl Peers for P2pPeers {
    async fn headers(&self, start: H256, count: u64) -> Result<Reply<Vec<BlockHeader>>, NetError> {
        let (peer, mut connection, _permit) = self.eth_peer().await?;
        let id: u64 = rand::random();
        let message = Message::GetBlockHeaders(GetBlockHeaders {
            id,
            startblock: HashOrNumber::Hash(start),
            limit: count.clamp(1, 1024),
            skip: 0,
            reverse: true,
        });
        match self.request(peer, &mut connection, message).await? {
            Message::BlockHeaders(headers) if headers.id == id => {
                if headers.block_headers.is_empty() {
                    // A peer without the block is of no use for this chain right now; let
                    // selection drift to others instead of asking it again.
                    let _ = self.peer_table.record_failure(peer);
                }
                Ok(Reply {
                    peer,
                    value: headers.block_headers,
                })
            }
            other => Err(self.mismatch(peer, &kind_of(&other))),
        }
    }

    async fn account_range(
        &self,
        request: AccountRangeRequest,
    ) -> Result<Reply<AccountReply>, NetError> {
        let (peer, mut connection, _permit) = self.snap_peer().await?;
        let id: u64 = rand::random();
        let message = Message::GetAccountRange(GetAccountRange {
            id,
            root_hash: request.root,
            starting_hash: request.key,
            limit_hash: request.key,
            response_bytes: MAX_RESPONSE_BYTES,
        });
        match self.request(peer, &mut connection, message).await? {
            Message::AccountRange(range) if range.id == id => Ok(Reply {
                peer,
                value: AccountReply {
                    accounts: range
                        .accounts
                        .into_iter()
                        .map(|unit| (unit.hash, unit.account))
                        .collect(),
                    proof: range.proof.into_iter().map(|node| node.to_vec()).collect(),
                },
            }),
            other => Err(self.mismatch(peer, &kind_of(&other))),
        }
    }

    async fn storage_range(
        &self,
        request: StorageRangeRequest,
    ) -> Result<Reply<StorageReply>, NetError> {
        let (peer, mut connection, _permit) = self.snap_peer().await?;
        let id: u64 = rand::random();
        let message = Message::GetStorageRanges(GetStorageRanges {
            id,
            root_hash: request.root,
            account_hashes: vec![request.account],
            starting_hash: request.slot,
            limit_hash: request.slot,
            response_bytes: MAX_RESPONSE_BYTES,
        });
        match self.request(peer, &mut connection, message).await? {
            Message::StorageRanges(ranges) if ranges.id == id => {
                if ranges.slots.len() > 1 {
                    return Err(self.mismatch(peer, "storage for more than one account"));
                }
                Ok(Reply {
                    peer,
                    value: StorageReply {
                        slots: ranges
                            .slots
                            .into_iter()
                            .next()
                            .unwrap_or_default()
                            .into_iter()
                            .map(|slot| (slot.hash, slot.data))
                            .collect(),
                        proof: ranges.proof.into_iter().map(|node| node.to_vec()).collect(),
                    },
                })
            }
            other => Err(self.mismatch(peer, &kind_of(&other))),
        }
    }

    async fn byte_codes(&self, hashes: Vec<H256>) -> Result<Reply<Vec<Option<Bytes>>>, NetError> {
        let (peer, mut connection, _permit) = self.snap_peer().await?;
        let id: u64 = rand::random();
        let wanted = hashes.len();
        let message = Message::GetByteCodes(GetByteCodes {
            id,
            hashes,
            bytes: MAX_RESPONSE_BYTES,
        });
        match self.request(peer, &mut connection, message).await? {
            Message::ByteCodes(codes) if codes.id == id => {
                if codes.codes.len() > wanted {
                    return Err(self.mismatch(peer, "more codes than requested"));
                }
                let mut value: Vec<Option<Bytes>> = codes.codes.into_iter().map(Some).collect();
                value.resize(wanted, None);
                Ok(Reply { peer, value })
            }
            other => Err(self.mismatch(peer, &kind_of(&other))),
        }
    }

    /// snap/2 first, then eth/71. The eth/71 path already checks each list against its
    /// header and reports a mismatch as unavailable, which `verify::bal` then reports as a
    /// peer lacking it; the snap/2 path is checked by `verify::bal` itself.
    async fn block_access_lists(
        &self,
        headers: Vec<BlockHeader>,
    ) -> Result<Reply<Vec<Option<BlockAccessList>>>, NetError> {
        let hashes: Vec<H256> = headers.iter().map(BlockHeader::hash).collect();
        let mut handler = self.peer_handler.clone();
        match handler.request_snap2_bals(&hashes, &[]).await {
            Ok(Some((bals, peer))) if bals.iter().any(Option::is_some) => {
                let mut value = bals;
                value.resize(hashes.len(), None);
                return Ok(Reply { peer, value });
            }
            Ok(_) => debug!("no snap/2 peer served the block access lists; trying eth/71"),
            Err(err) => debug!(%err, "snap/2 block access list request failed; trying eth/71"),
        }
        match handler.request_block_access_lists(&headers).await {
            Ok(Some(bals)) => {
                let mut value = bals;
                value.resize(hashes.len(), None);
                Ok(Reply {
                    peer: H256::zero(),
                    value,
                })
            }
            Ok(None) => Err(NetError::NoPeer),
            Err(err) => Err(NetError::Rejected(err.to_string())),
        }
    }

    async fn bodies(&self, headers: Vec<BlockHeader>) -> Result<Reply<Vec<BlockBody>>, NetError> {
        let mut handler = self.peer_handler.clone();
        match handler.request_block_bodies(&headers).await {
            Ok(Some(bodies)) => Ok(Reply {
                peer: H256::zero(),
                value: bodies,
            }),
            Ok(None) => Err(NetError::NoPeer),
            Err(err) => Err(NetError::Rejected(err.to_string())),
        }
    }

    async fn receipts(&self, header: BlockHeader) -> Result<Reply<Vec<Receipt>>, NetError> {
        let mut handler = self.peer_handler.clone();
        match handler
            .request_receipts(std::slice::from_ref(&header))
            .await
        {
            Ok(Some(mut receipts)) if !receipts.is_empty() => Ok(Reply {
                peer: H256::zero(),
                value: receipts.remove(0),
            }),
            Ok(_) => Err(NetError::NoPeer),
            Err(err) => Err(NetError::Rejected(err.to_string())),
        }
    }

    async fn broadcast(&self, tx: Transaction) -> Result<usize, NetError> {
        let peers = self
            .peer_table
            .get_best_n_peers(SUPPORTED_ETH_CAPABILITIES.to_vec(), BROADCAST_PEERS)
            .await
            .map_err(|e| NetError::Rejected(e.to_string()))?;
        if peers.is_empty() {
            return Err(NetError::NoPeer);
        }
        let mut sent = 0;
        for (peer, mut connection, _permit) in peers {
            let message = Message::Transactions(Transactions::new(vec![tx.clone()]));
            match connection.outgoing_message(message).await {
                Ok(()) => sent += 1,
                Err(err) => debug!(%peer, %err, "transaction not sent"),
            }
        }
        Ok(sent)
    }

    fn penalize(&self, peer: PeerKey) {
        if peer.is_zero() {
            return;
        }
        if let Err(err) = self.peer_table.record_critical_failure(peer) {
            warn!(%peer, %err, "could not penalize peer");
        }
    }

    /// Writes the held headers into the node's store and points its fork choice at the
    /// anchor, so the eth handshake and range announcements carry the verified head.
    async fn announce(&self, store: &PartialStore) {
        let headers: Vec<BlockHeader> = store.headers().map(|h| h.header().clone()).collect();
        let anchor = store.anchor();
        let canonical: Vec<(u64, H256)> = store.headers().map(|h| (h.number(), h.hash())).collect();
        if let Err(err) = self.store.add_block_headers(headers).await {
            debug!(%err, "could not store headers for the handshake");
            return;
        }
        if let Err(err) = self
            .store
            .forkchoice_update(canonical, anchor.number(), anchor.hash(), None, None)
            .await
        {
            debug!(%err, "could not update the fork choice for the handshake");
        }
        *self.peer_handler.latest_fcu_head.lock().await = anchor.hash();
    }

    async fn census(&self) -> Census {
        let state_peers = self
            .peer_table
            .peer_count_by_capabilities(SUPPORTED_SNAP_CAPABILITIES.to_vec())
            .await
            .unwrap_or(0);
        let eth_peers = self
            .peer_table
            .peer_count_by_capabilities(SUPPORTED_ETH_CAPABILITIES.to_vec())
            .await
            .unwrap_or(0);
        Census {
            state_peers,
            eth_peers,
        }
    }
}

/// The variant name of a message, for diagnostics.
fn kind_of(message: &Message) -> String {
    let text = format!("{message:?}");
    text.split(['(', ' ', '{'])
        .next()
        .unwrap_or("unknown")
        .to_string()
}

/// A shareable `Peers`.
pub fn shared(peers: P2pPeers) -> Arc<dyn Peers> {
    Arc::new(peers)
}
