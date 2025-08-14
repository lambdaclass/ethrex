use std::{
    net::IpAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ethrex_common::{H256, H512};
use keccak_hash::keccak;
use secp256k1::{PublicKey, SecretKey};

use crate::peer_handler::DumpError;

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

pub fn get_account_storages_snapshots_dir() -> Option<String> {
    let home_dir = std::env::home_dir()?;
    let home_dir = home_dir.to_str()?;
    let account_storages_snapshots_dir =
        format!("{home_dir}/.local/share/ethrex/account_storages_snapshots");
    Some(account_storages_snapshots_dir)
}

pub fn get_account_state_snapshots_dir() -> Option<String> {
    let home_dir = std::env::home_dir()?;
    let home_dir = home_dir.to_str()?;
    let account_state_snapshots_dir =
        format!("{home_dir}/.local/share/ethrex/account_state_snapshots");
    Some(account_state_snapshots_dir)
}

pub fn get_account_state_snapshot_file(directory: String, chunk_index: u64) -> String {
    format!("{directory}/account_state_chunk.rlp.{chunk_index}")
}

pub fn get_account_storages_snapshot_file(directory: String, chunk_index: u64) -> String {
    format!("{directory}/account_storages_chunk.rlp.{chunk_index}")
}

pub fn dump_to_file(path: String, contents: Vec<u8>) -> Result<(), DumpError> {
    std::fs::write(&path, &contents)
        .inspect_err(|err| {
            tracing::error!("Failed to write accounts to path {}. Error: {}", &path, err)
        })
        .map_err(|err| DumpError {
            path,
            contents,
            error: err.kind(),
        })
}
