use super::{message::Message, p2p::DisconnectReason};
use crate::peer_table::PeerTableError;
use aes::cipher::InvalidLength;
use ethrex_blockchain::error::{ChainError, MempoolError};
use ethrex_rlp::error::{RLPDecodeError, RLPEncodeError};
use ethrex_storage::error::StoreError;
#[cfg(feature = "l2")]
use ethrex_storage_rollup::RollupStoreError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptographyError {
    #[error("Invalid key: {0}")]
    InvalidKey(String),
    #[error("Invalid generated secret: {0}")]
    InvalidGeneratedSecret(String),
    #[error("Couldn't get keys from shared secret: {0}")]
    CouldNotGetKeyFromSecret(String),
}

// TODO improve errors
#[derive(Debug, Error)]
pub enum PeerConnectionError {
    #[error("{0}")]
    HandshakeError(String),
    #[error("Invalid connection state: {0}")]
    StateError(String),
    #[error("No matching capabilities")]
    NoMatchingCapabilities,
    #[error("Too many peers")]
    TooManyPeers,
    #[error("Peer disconnected")]
    Disconnected,
    #[error("Disconnect requested: {0}")]
    DisconnectReceived(DisconnectReason),
    #[error("Disconnect sent: {0}")]
    DisconnectSent(DisconnectReason),
    #[error("Not Found: {0}")]
    NotFound(String),
    #[error("Invalid peer id")]
    InvalidPeerId,
    #[error("Invalid recovery id")]
    InvalidRecoveryId,
    #[error("Invalid message length")]
    InvalidMessageLength,
    #[error("Request id not present: {0}")]
    ExpectedRequestId(String),
    #[error("Cannot handle message: {0}")]
    MessageNotHandled(String),
    #[error("Bad Request: {0}")]
    BadRequest(String),
    #[error(transparent)]
    RLPDecodeError(#[from] RLPDecodeError),
    #[error(transparent)]
    RLPEncodeError(#[from] RLPEncodeError),
    #[error(transparent)]
    StoreError(#[from] StoreError),
    #[error(transparent)]
    #[cfg(feature = "l2")]
    RollupStoreError(#[from] RollupStoreError),
    #[error("Error in cryptographic library: {0}")]
    CryptographyError(String),
    #[error("Failed to broadcast msg: {0}")]
    BroadcastError(String),
    #[error("RecvError: {0}")]
    RecvError(String),
    #[error("Failed to send msg: {0}")]
    SendMessage(String),
    #[error("Error when inserting transaction in the mempool: {0}")]
    MempoolError(#[from] MempoolError),
    #[error("Error when adding a block to the blockchain: {0}")]
    BlockchainError(#[from] ChainError),
    #[error("Io Error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Failed to decode message due to invalid frame: {0}")]
    InvalidMessageFrame(String),
    #[error("Failed due to an internal error: {0}")]
    InternalError(String),
    #[error("Incompatible Protocol")]
    IncompatibleProtocol,
    #[error("Invalid block range")]
    InvalidBlockRange,
    #[error("An L2 functionality was used but it was not previously negotiated")]
    L2CapabilityNotNegotiated,
    #[error("Received invalid block range update")]
    InvalidBlockRangeUpdate,
    #[error(transparent)]
    PeerTableError(#[from] PeerTableError),
    #[error("Request timeouted")]
    Timeout,
    #[error("Unexpected response: Expected {0}, got {1}")]
    UnexpectedResponse(String, String),
}

// tokio::sync::mpsc::error::SendError<Message> is too large to be part of the RLPxError enum directly
// so we will instead save the error's display message
impl From<tokio::sync::mpsc::error::SendError<Message>> for PeerConnectionError {
    fn from(value: tokio::sync::mpsc::error::SendError<Message>) -> Self {
        Self::SendMessage(value.to_string())
    }
}

// Grouping all cryptographic related errors in a single CryptographicError variant
// We can improve this to individual errors if required
impl From<secp256k1::Error> for PeerConnectionError {
    fn from(e: secp256k1::Error) -> Self {
        PeerConnectionError::CryptographyError(e.to_string())
    }
}

impl From<InvalidLength> for PeerConnectionError {
    fn from(e: InvalidLength) -> Self {
        PeerConnectionError::CryptographyError(e.to_string())
    }
}

impl From<aes::cipher::StreamCipherError> for PeerConnectionError {
    fn from(e: aes::cipher::StreamCipherError) -> Self {
        PeerConnectionError::CryptographyError(e.to_string())
    }
}

impl From<tokio::sync::broadcast::error::RecvError> for PeerConnectionError {
    fn from(e: tokio::sync::broadcast::error::RecvError) -> Self {
        PeerConnectionError::RecvError(e.to_string())
    }
}

impl From<tokio::sync::oneshot::error::RecvError> for PeerConnectionError {
    fn from(e: tokio::sync::oneshot::error::RecvError) -> Self {
        PeerConnectionError::RecvError(e.to_string())
    }
}
