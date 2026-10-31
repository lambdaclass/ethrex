//! The network seam: untrusted requests and replies in the node's own shapes.
//!
//! Everything returned here is unverified wire data. [`crate::fetch`] turns it into verified
//! values through [`crate::verify`]. None of the errors say anything about chain state.

use async_trait::async_trait;
use bytes::Bytes;
use ethrex_common::H256;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::{BlockBody, BlockHeader, Receipt, Transaction};

use crate::state::Store;
use crate::verify::{AccountRangeRequest, AccountReply, StorageRangeRequest, StorageReply};

/// Identifies the peer a reply came from, for blame.
pub type PeerKey = H256;

/// A reply and who sent it.
#[derive(Clone, Debug)]
pub struct Reply<T> {
    /// The sender.
    pub peer: PeerKey,
    /// The reply.
    pub value: T,
}

/// Peer counts, for status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census {
    /// Peers that serve state (snap).
    pub state_peers: usize,
    /// Peers that serve headers, bodies, receipts and BALs (eth).
    pub eth_peers: usize,
}

/// A request did not complete. None of these say anything about chain state.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NetError {
    /// No connected peer can serve the request.
    #[error("no peer can serve the request")]
    NoPeer,
    /// The peer did not answer in time.
    #[error("request timed out")]
    Timeout,
    /// The peer answered with something that is not a reply to the request.
    #[error("peer rejected the request: {0}")]
    Rejected(String),
    /// The node is shutting down.
    #[error("node stopped")]
    Stopped,
}

/// What the node asks peers for.
#[async_trait]
pub trait Peers: Send + Sync + 'static {
    /// Up to `count` headers falling from `start` inclusive, newest first.
    async fn headers(&self, start: H256, count: u64) -> Result<Reply<Vec<BlockHeader>>, NetError>;

    /// A single-key `GetAccountRange`.
    async fn account_range(
        &self,
        request: AccountRangeRequest,
    ) -> Result<Reply<AccountReply>, NetError>;

    /// A single-key `GetStorageRanges` for one account.
    async fn storage_range(
        &self,
        request: StorageRangeRequest,
    ) -> Result<Reply<StorageReply>, NetError>;

    /// `GetByteCodes`, aligned to `hashes`; `None` where the peer returned nothing.
    async fn byte_codes(&self, hashes: Vec<H256>) -> Result<Reply<Vec<Option<Bytes>>>, NetError>;

    /// Block access lists for `headers`, aligned; `None` where the peer lacks one.
    async fn block_access_lists(
        &self,
        headers: Vec<BlockHeader>,
    ) -> Result<Reply<Vec<Option<BlockAccessList>>>, NetError>;

    /// Bodies for `headers`, aligned; a short reply means the peer capped it.
    async fn bodies(&self, headers: Vec<BlockHeader>) -> Result<Reply<Vec<BlockBody>>, NetError>;

    /// The receipts of one block.
    async fn receipts(&self, header: BlockHeader) -> Result<Reply<Vec<Receipt>>, NetError>;

    /// Sends a transaction to peers. Returns how many received it.
    async fn broadcast(&self, tx: Transaction) -> Result<usize, NetError>;

    /// Lowers a peer's standing after it sent bad data.
    fn penalize(&self, peer: PeerKey);

    /// Tells peers which block the node considers its head.
    async fn announce(&self, store: &Store);

    /// Peer counts.
    async fn census(&self) -> Census;
}
