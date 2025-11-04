use crate::peer_handler::DumpError;
use ethrex_common::{H256, H512, U256, types::AccountState, utils::keccak};
use ethrex_rlp::encode::RLPEncode;
use secp256k1::{PublicKey, SecretKey};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
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

/// Deletes the snap folders needed for downloading the leaves during the initial
/// step of snap sync.
pub fn delete_leaves_folder(datadir: &Path) {
    // We ignore the errors because this can happen when the folders don't exist
    let _ = std::fs::remove_dir_all(get_account_state_snapshots_dir(datadir));
    let _ = std::fs::remove_dir_all(get_account_storages_snapshots_dir(datadir));
    let _ = std::fs::remove_dir_all(get_code_hashes_snapshots_dir(datadir));
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

pub fn get_code_hashes_snapshots_dir(datadir: &Path) -> PathBuf {
    datadir.join("bytecode_hashes_snapshots")
}

pub fn get_code_hashes_snapshot_file(directory: &Path, chunk_index: u64) -> PathBuf {
    directory.join(format!("bytecode_hashes_chunk.rlp.{chunk_index}"))
}

pub fn dump_to_file(path: &Path, contents: Vec<u8>) -> Result<(), DumpError> {
    std::fs::write(path, &contents)
        .inspect_err(|err| error!(%err, ?path, "Failed to dump snapshot to file"))
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
    dump_to_file(
        path,
        storages
            .into_iter()
            .map(|accounts_with_slots| (accounts_with_slots.accounts, accounts_with_slots.storages))
            .collect::<Vec<_>>()
            .encode_to_vec(),
    )
}
