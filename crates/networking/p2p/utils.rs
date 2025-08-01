use std::{
    net::IpAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ethrex_common::{H256, H512, types::BlockHeader};
use ethrex_rlp::error::RLPDecodeError;
use ethrex_trie::Node;
use keccak_hash::keccak;
use secp256k1::{PublicKey, SecretKey};
use spawned_concurrency::error::GenServerError;
use tracing::info;

use crate::{
    kademlia::PeerChannels,
    rlpx::{Message, connection::server::CastMessage, message::RLPxMessage, snap::TrieNodes},
};

/// Computes the node_id from a public key (aka computes the Keccak256 hash of the given public key)
pub fn node_id(public_key: &H512) -> H256 {
    keccak(public_key)
}

pub fn current_unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn get_msg_expiration_from_seconds(seconds: u64) -> u64 {
    (SystemTime::now() + Duration::from_secs(seconds))
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn is_msg_expired(expiration: u64) -> bool {
    // this cast to a signed integer is needed as the rlp decoder doesn't take into account the sign
    // otherwise if a msg contains a negative expiration, it would pass since as it would wrap around the u64.
    (expiration as i64) < (current_unix_time() as i64)
}

pub fn public_key_from_signing_key(signer: &SecretKey) -> H512 {
    let public_key = PublicKey::from_secret_key(secp256k1::SECP256K1, signer);
    let encoded = public_key.serialize_uncompressed();
    H512::from_slice(&encoded[1..])
}

pub fn unmap_ipv4in6_address(addr: IpAddr) -> IpAddr {
    if let IpAddr::V6(v6_addr) = addr {
        if let Some(v4_addr) = v6_addr.to_ipv4_mapped() {
            return IpAddr::V4(v4_addr);
        }
    }
    addr
}

/// Validates the block headers received from a peer by checking that the parent hash of each header
/// matches the hash of the previous one, i.e. the headers are chained
pub fn are_block_headers_chained(block_headers: &[BlockHeader]) -> bool {
    block_headers
        .windows(2)
        .all(|headers| headers[1].parent_hash == headers[0].hash())
}

pub fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    format!("{hours:02}h {minutes:02}m {seconds:02}s")
}

/// TODO: make it more generic
pub async fn send_message_and_wait_for_response(
    peer_channel: &mut PeerChannels,
    message: Message,
    request_id: u64,
) -> Result<Vec<Node>, SendMessageError> {
    let mut receiver = peer_channel.receiver.lock().await;
    peer_channel
        .connection
        .cast(CastMessage::BackendMessage(message))
        .await
        .map_err(SendMessageError::GenServerError)?;
    let nodes = tokio::time::timeout(Duration::from_secs(7), async move {
        loop {
            let Some(resp) = receiver.recv().await else {
                return None;
            };
            if let Message::TrieNodes(TrieNodes { id, nodes }) = resp {
                if id == request_id {
                    return Some(nodes);
                }
            }
        }
    })
    .await
    .map_err(|_| SendMessageError::PeerTimeout)?
    .ok_or_else(|| SendMessageError::PeerDisconnected)?;

    nodes
        .iter()
        .map(|node| Node::decode_raw(node))
        .collect::<Result<Vec<_>, _>>()
        .map_err(SendMessageError::RLPDecodeError)
}

// TODO: find a better name for this type
#[derive(thiserror::Error, Debug)]
pub enum SendMessageError {
    #[error("Peer timed out")]
    PeerTimeout,
    #[error("GenServerError")]
    GenServerError(GenServerError),
    #[error("Peer disconnected")]
    PeerDisconnected,
    #[error("RLP decode error")]
    RLPDecodeError(RLPDecodeError),
}
