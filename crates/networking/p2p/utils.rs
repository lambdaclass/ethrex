use std::{
    net::IpAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ethrex_common::{H256, H512};
use k256::{PublicKey, ecdsa::SigningKey, elliptic_curve::sec1::ToEncodedPoint};
use keccak_hash::keccak;

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

pub fn public_key_from_signing_key(signer: &SigningKey) -> H512 {
    let public_key = PublicKey::from(signer.verifying_key());
    let encoded = public_key.to_encoded_point(false);
    H512::from_slice(&encoded.as_bytes()[1..])
}

pub fn unmap_ipv4in6_address(addr: IpAddr) -> IpAddr {
    if let IpAddr::V6(v6_addr) = addr {
        if let Some(v4_addr) = v6_addr.to_ipv4_mapped() {
            return IpAddr::V4(v4_addr);
        }
    }
    addr
}
