use ethrex_blockchain::error::{ChainError, MempoolError};
use ethrex_rlp::error::{RLPDecodeError, RLPEncodeError};
use ethrex_storage::error::StoreError;
use ethrex_storage_rollup::RollupStoreError;
use thiserror::Error;
use tokio::sync::broadcast::error::RecvError;

use super::{message::Message, p2p::DisconnectReason};

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
pub enum RLPxError {
    #[error("{0}")]
    HandshakeError(String),
    #[error("Invalid connection state: {0}")]
    StateError(String),
    #[error("No matching capabilities")]
    NoMatchingCapabilities(),
    #[error("Peer disconnected")]
    Disconnected(),
    #[error("Disconnect requested: {0}")]
    DisconnectReceived(DisconnectReason),
    #[error("Disconnect sent: {0}")]
    DisconnectSent(DisconnectReason),
    #[error("Not Found: {0}")]
    NotFound(String),
    #[error("Invalid peer id")]
    InvalidPeerId(),
    #[error("Invalid recovery id")]
    InvalidRecoveryId(),
    #[error("Invalid message length")]
    InvalidMessageLength(),
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
    RollupStoreError(#[from] RollupStoreError),
    #[error("Error in cryptographic library: {0}")]
    CryptographyError(String),
    #[error("Failed to broadcast msg: {0}")]
    BroadcastError(String),
    #[error(transparent)]
    RecvError(#[from] RecvError),
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
}

// tokio::sync::mpsc::error::SendError<Message> is too large to be part of the RLPxError enum directly
// so we will instead save the error's display message
impl From<tokio::sync::mpsc::error::SendError<Message>> for RLPxError {
    fn from(value: tokio::sync::mpsc::error::SendError<Message>) -> Self {
        Self::SendMessage(value.to_string())
    }
}

// Grouping all cryptographic related errors in a single CryptographicError variant
// We can improve this to individual errors if required
impl From<secp256k1::Error> for RLPxError {
    fn from(e: secp256k1::Error) -> Self {
        RLPxError::CryptographyError(e.to_string())
    }
}

impl From<sha3::digest::InvalidLength> for RLPxError {
    fn from(e: sha3::digest::InvalidLength) -> Self {
        RLPxError::CryptographyError(e.to_string())
    }
}

impl From<aes::cipher::StreamCipherError> for RLPxError {
    fn from(e: aes::cipher::StreamCipherError) -> Self {
        RLPxError::CryptographyError(e.to_string())
    }
}
