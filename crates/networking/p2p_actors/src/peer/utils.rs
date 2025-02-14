use ethrex_core::H512;
use k256::{
    elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint},
    EncodedPoint, PublicKey,
};

/// Computes public key from recipient id.
/// The node ID is the uncompressed public key of a node, with the first byte omitted (0x04).
pub fn id2pubkey(id: H512) -> Option<PublicKey> {
    let point = EncodedPoint::from_untagged_bytes(&id.0.into());
    PublicKey::from_encoded_point(&point).into_option()
}

/// Computes recipient id from public key.
pub fn pubkey2id(pk: &PublicKey) -> H512 {
    let encoded = pk.to_encoded_point(false);
    let bytes = encoded.as_bytes();
    debug_assert_eq!(bytes[0], 4);
    H512::from_slice(&bytes[1..])
}
