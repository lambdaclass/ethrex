use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ethrex_common::utils::keccak;
use ethrex_common::{H256, H512, U256, types::AccountState};
use ethrex_rlp::{encode::RLPEncode, error::RLPDecodeError};
use ethrex_trie::Node;
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

pub fn get_account_storages_snapshots_dir(datadir: &Path) -> PathBuf {
    datadir.join("account_storages_snapshots")
}

pub fn get_account_state_snapshots_dir(datadir: &Path) -> PathBuf {
    datadir.join("account_state_snapshots")
}

pub fn get_rocksdb_temp_accounts_dir(datadir: &Path) -> PathBuf {
    datadir.join("temp_acc_dir")
}

pub fn get_rocksdb_temp_storage_dir(datadir: &Path) -> PathBuf {
    datadir.join("temp_storage_dir")
}

pub fn get_account_state_snapshot_file(directory: &Path, chunk_index: u64) -> PathBuf {
    directory.join(format!("account_state_chunk.rlp.{chunk_index}"))
}

pub fn get_account_storages_snapshot_file(directory: &Path, chunk_index: u64) -> PathBuf {
    directory.join(format!("account_storages_chunk.rlp.{chunk_index}"))
}

#[cfg(feature = "rocksdb")]
pub fn dump_accounts_to_rocks_db(
    path: &Path,
    mut contents: Vec<(H256, AccountState)>,
) -> Result<(), rocksdb::Error> {
    contents.sort_by_key(|(k, _)| *k);
    contents.dedup_by_key(|(k, _)| {
        let mut buf = [0u8; 32];
        buf[..32].copy_from_slice(&k.0);
        buf
    });
    let mut buffer: Vec<u8> = Vec::new();
    let writer_options = rocksdb::Options::default();
    let mut writer = rocksdb::SstFileWriter::create(&writer_options);
    writer.open(std::path::Path::new(&path))?;
    for (key, acccount) in contents {
        buffer.clear();
        acccount.encode(&mut buffer);
        writer.put(key.0.as_ref(), buffer.as_slice())?;
    }
    writer.finish()
}

#[cfg(feature = "rocksdb")]
pub fn dump_storages_to_rocks_db(
    path: &Path,
    mut contents: Vec<(H256, H256, U256)>,
) -> Result<(), rocksdb::Error> {
    use tracing::info;

    contents.sort();
    contents.dedup_by_key(|(k0, k1, _)| {
        let mut buffer = [0_u8; 64];
        buffer[0..32].copy_from_slice(&k0.0);
        buffer[32..64].copy_from_slice(&k1.0);
        buffer
    });
    let writer_options = rocksdb::Options::default();
    let mut writer = rocksdb::SstFileWriter::create(&writer_options);
    let mut buffer_key = [0_u8; 64];
    let mut buffer_storage: Vec<u8> = Vec::new();
    writer.open(std::path::Path::new(&path))?;
    for (account, slot_hash, slot_value) in contents {
        buffer_key[0..32].copy_from_slice(&account.0);
        buffer_key[32..64].copy_from_slice(&slot_hash.0);
        buffer_storage.clear();
        slot_value.encode(&mut buffer_storage);
        writer.put(buffer_key.as_ref(), buffer_storage.as_slice())?;
    }
    writer.finish()
}

pub fn get_code_hashes_snapshots_dir(datadir: &Path) -> PathBuf {
    datadir.join("bytecode_hashes_snapshots")
}

pub fn get_code_hashes_snapshot_file(directory: &Path, chunk_index: u64) -> PathBuf {
    directory.join(format!("bytecode_hashes_chunk.rlp.{chunk_index}"))
}

pub fn dump_to_file(path: &Path, contents: Vec<u8>) -> Result<(), DumpError> {
    std::fs::write(path, &contents)
        .inspect_err(|err| tracing::error!(%err, ?path, "Failed to dump snapshot to file"))
        .map_err(|err| DumpError {
            path: path.to_path_buf(),
            contents,
            error: err.kind(),
        })
}

pub fn dump_accounts_to_file(
    path: &Path,
    accounts: Vec<(H256, AccountState)>,
) -> Result<(), DumpError> {
    #[cfg(feature = "rocksdb")]
    return dump_accounts_to_rocks_db(path, accounts)
        .inspect_err(|err| error!("Rocksdb writing stt error {err:?}"))
        .map_err(|_| DumpError {
            path: path.to_path_buf(),
            contents: Vec::new(),
            error: std::io::ErrorKind::Other,
        });
    #[cfg(not(feature = "rocksdb"))]
    dump_to_file(path, accounts.encode_to_vec())
}

/// Struct representing the storage slots of certain accounts that share the same storage root
pub struct AccountsWithStorage {
    /// Accounts with the same storage root
    pub accounts: Vec<H256>,
    /// All slots in the trie from the accounts
    pub storages: Vec<(H256, U256)>,
}

pub fn dump_storages_to_file(
    path: &Path,
    storages: Vec<AccountsWithStorage>,
) -> Result<(), DumpError> {
    #[cfg(feature = "rocksdb")]
    return dump_storages_to_rocks_db(
        path,
        storages
            .into_iter()
            .flat_map(|accounts_with_slots| {
                accounts_with_slots
                    .accounts
                    .into_iter()
                    .map(|hash| {
                        accounts_with_slots
                            .storages
                            .iter()
                            .map(move |(slot_hash, slot_value)| (hash, *slot_hash, *slot_value))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
            .flatten()
            .collect::<Vec<_>>(),
    )
    .inspect_err(|err| error!("Rocksdb writing stt error {err:?}"))
    .map_err(|_| DumpError {
        path: path.to_path_buf(),
        contents: Vec::new(),
        error: std::io::ErrorKind::Other,
    });

    #[cfg(not(feature = "rocksdb"))]
    dump_to_file(
        path,
        storages
            .into_iter()
            .map(|accounts_with_slots| (accounts_with_slots.accounts, accounts_with_slots.storages))
            .collect::<Vec<_>>()
            .encode_to_vec(),
    )
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
