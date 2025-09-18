use std::{
    net::IpAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ethrex_common::{H256, H512, U256, types::AccountState};
use ethrex_rlp::{encode::RLPEncode, error::RLPDecodeError};
use ethrex_trie::Node;
use keccak_hash::keccak;
use secp256k1::{PublicKey, SecretKey};
use spawned_concurrency::error::GenServerError;

use crate::peer_handler::DumpError;
use crate::{
    kademlia::PeerChannels,
    rlpx::{Message, connection::server::CastMessage, snap::TrieNodes},
};
use tracing::error;

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

pub fn get_account_storages_snapshots_dir(datadir: &String) -> String {
    format!("{datadir}/account_storages_snapshots")
}

pub fn get_account_state_snapshots_dir(datadir: &String) -> String {
    format!("{datadir}/account_state_snapshots")
}

pub fn get_rocksdb_temp_accounts_dir(datadir: &String) -> String {
    format!("{datadir}/temp_acc_dir")
}

pub fn get_rocksdb_temp_storage_dir(datadir: &String) -> String {
    format!("{datadir}/temp_storage_dir")
}

pub fn get_account_state_snapshot_file(directory: String, chunk_index: u64) -> String {
    format!("{directory}/account_state_chunk.rlp.{chunk_index}")
}

pub fn get_account_storages_snapshot_file(directory: String, chunk_index: u64) -> String {
    format!("{directory}/account_storages_chunk.rlp.{chunk_index}")
}

#[cfg(feature = "rocksdb")]
pub fn dump_to_rocks_db(
    path: String,
    mut contents: Vec<(Vec<u8>, Vec<u8>)>,
) -> Result<(), rocksdb::Error> {
    contents.sort();
    contents.dedup_by_key(|(k, _)| {
        let mut buf = [0u8; 64];
        buf[..k.len()].copy_from_slice(k);
        buf
    });
    let writer_options = rocksdb::Options::default();
    let mut writer = rocksdb::SstFileWriter::create(&writer_options);
    writer.open(std::path::Path::new(&path))?;
    for values in contents {
        writer.put(values.0, values.1)?;
    }
    writer.finish()
}

pub fn dump_to_file(path: String, contents: Vec<u8>) -> Result<(), DumpError> {
    std::fs::write(&path, &contents)
        .inspect_err(|err| {
            tracing::error!("Failed to write snapshot to path {}. Error: {}", &path, err)
        })
        .map_err(|err| DumpError {
            path,
            contents,
            error: err.kind(),
        })
}

pub fn dump_accounts_to_file(
    path: String,
    accounts: Vec<(H256, AccountState)>,
) -> Result<(), DumpError> {
    cfg_if::cfg_if! {
        if #[cfg(feature = "rocksdb")] {
            dump_to_rocks_db(
                path.clone(),
                accounts
                    .into_iter()
                    .map(|(hash, state)| (hash.0.to_vec(), state.encode_to_vec())
                ).collect::<Vec<_>>()
            )
                .inspect_err(|err| error!("Rocksdb writing stt error {err:?}"))
                .map_err(|_| DumpError {
                    path,
                    contents: Vec::new(),
                    error: std::io::ErrorKind::Other,
                })
        } else {
            dump_to_file(path, accounts.encode_to_vec())
        }
    }
}

pub fn dump_storages_to_file(
    path: String,
    storages: Vec<(H256, Vec<(H256, U256)>)>,
) -> Result<(), DumpError> {
    cfg_if::cfg_if! {
        if #[cfg(feature = "rocksdb")] {
            dump_to_rocks_db(
                path.clone(),
                storages
                    .into_iter()
                    .flat_map(|(hash, slots)| {
                        slots.into_iter().map(move |(slot_hash, slot_value)| {
                            let key = [hash.as_bytes(), slot_hash.as_bytes()].concat();
                            (key, slot_value.encode_to_vec())
                        })
                    }).collect::<Vec<_>>()
            )
                .inspect_err(|err| error!("Rocksdb writing stt error {err:?}"))
                .map_err(|_| DumpError {
                    path,
                    contents: Vec::new(),
                    error: std::io::ErrorKind::Other,
                })
        } else {
            dump_to_file(path, storages.encode_to_vec())
        }
    }
}

/// TODO: make it more generic
pub async fn send_message_and_wait_for_response(
    peer_channel: &mut PeerChannels,
    message: Message,
    request_id: u64,
) -> Result<Vec<Node>, SendMessageError> {
    let receiver = peer_channel
        .receiver
        .try_lock()
        .map_err(|_| SendMessageError::PeerBusy)?;
    peer_channel
        .connection
        .cast(CastMessage::BackendMessage(message))
        .await
        .map_err(SendMessageError::GenServerError)?;
    let nodes = tokio::time::timeout(
        Duration::from_secs(7),
        receive_trienodes(receiver, request_id),
    )
    .await
    .map_err(|_| SendMessageError::PeerTimeout)?
    .ok_or(SendMessageError::PeerDisconnected)?;

    nodes
        .nodes
        .iter()
        .map(|node| Node::decode_raw(node))
        .collect::<Result<Vec<_>, _>>()
        .map_err(SendMessageError::RLPDecodeError)
}

/// TODO: make it more generic
pub async fn send_trie_nodes_messages_and_wait_for_reply(
    peer_channel: &mut PeerChannels,
    message: Message,
    request_id: u64,
) -> Result<TrieNodes, SendMessageError> {
    let receiver = peer_channel
        .receiver
        .try_lock()
        .map_err(|_| SendMessageError::PeerBusy)?;
    peer_channel
        .connection
        .cast(CastMessage::BackendMessage(message))
        .await
        .map_err(SendMessageError::GenServerError)?;
    tokio::time::timeout(
        Duration::from_secs(7),
        receive_trienodes(receiver, request_id),
    )
    .await
    .map_err(|_| SendMessageError::PeerTimeout)?
    .ok_or(SendMessageError::PeerDisconnected)
}

async fn receive_trienodes(
    mut receiver: tokio::sync::MutexGuard<'_, spawned_rt::tasks::mpsc::Receiver<Message>>,
    request_id: u64,
) -> Option<TrieNodes> {
    loop {
        let resp = receiver.recv().await?;
        if let Message::TrieNodes(trie_nodes) = resp {
            if trie_nodes.id == request_id {
                return Some(trie_nodes);
            }
        }
    }
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
    #[error("Peer Busy")]
    PeerBusy,
    #[error("RLP decode error")]
    RLPDecodeError(RLPDecodeError),
}
