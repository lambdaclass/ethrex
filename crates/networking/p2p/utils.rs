use std::{
    io::ErrorKind,
    net::IpAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ethrex_common::{H256, H512};
use keccak_hash::keccak;
use secp256k1::{PublicKey, SecretKey};

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

#[derive(Clone, Debug)]
pub enum InternalStorageDumpError {
    DirectoryNotAccessible,
    FailedToCreateDirectory,
    IoRetriable(std::io::ErrorKind),
    IoNonRetriable(std::io::ErrorKind),
}

impl InternalStorageDumpError {
    fn retriable(&self) -> bool {
        match self {
            InternalStorageDumpError::DirectoryNotAccessible
            | InternalStorageDumpError::FailedToCreateDirectory
            | InternalStorageDumpError::IoNonRetriable(_) => false,
            InternalStorageDumpError::IoRetriable(_) => true,
        }
    }
}

impl From<std::io::Error> for InternalStorageDumpError {
    fn from(value: std::io::Error) -> Self {
        match value.kind() {
            ErrorKind::ResourceBusy | ErrorKind::Interrupted | ErrorKind::BrokenPipe => {
                Self::IoRetriable(value.kind())
            }
            other => Self::IoNonRetriable(other),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StorageDumpError {
    pub path: String,
    pub contents: Vec<u8>,
    pub reason: InternalStorageDumpError,
}

impl StorageDumpError {
    pub fn retriable(&self) -> bool {
        self.reason.retriable()
    }
}

// TODO: RELOCATE?
pub async fn dump_storage(path: String, contents: Vec<u8>) -> Result<String, StorageDumpError> {
    let directory = std::path::Path::new(&path)
        .parent()
        .expect("Failed to get parent directory");

    let exists = std::fs::exists(directory).map_err(|_| StorageDumpError {
        path: path.clone(),
        contents: contents.clone(),
        reason: InternalStorageDumpError::DirectoryNotAccessible,
    })?;

    if !exists {
        // Attempt to create needed directory in case it's not already there
        std::fs::create_dir_all(directory).map_err(|_| StorageDumpError {
            path: path.clone(),
            contents: contents.clone(),
            reason: InternalStorageDumpError::FailedToCreateDirectory,
        })?;
    }

    std::fs::write(path.clone(), contents.clone()).map_err(|e| StorageDumpError {
        path: path.clone(),
        contents,
        reason: e.into(),
    })?;

    Ok(path)
}
