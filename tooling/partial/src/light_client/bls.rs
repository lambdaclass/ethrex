//! BLS12-381 signature checks for sync committee attestations (the `min_pk` scheme:
//! public keys in G1, signatures in G2, as Ethereum uses).
//!
//! This mirrors ethlambda's `bls.rs` rather than linking `ethlambda-crypto`, which carries
//! the leanVM post-quantum stack this node has no use for.

use blst::BLST_ERROR;
use blst::min_pk::{PublicKey, Signature};

use super::types::{BlsPubkey, BlsSignature};

/// The hash-to-curve domain separation tag of Ethereum's BLS signatures.
pub const DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";

/// `bls.FastAggregateVerify(pubkeys, message, signature)`: whether `signature` is the
/// aggregate of signatures by every key in `pubkeys` over `message`.
///
/// Same checks as ethlambda's `ethlambda_crypto::bls::fast_aggregate_verify`: the signature
/// must deserialize to a point in the right subgroup, every key must pass `KeyValidate`, and
/// an empty key set fails (a sync committee attestation always has signers).
pub fn fast_aggregate_verify(
    pubkeys: &[BlsPubkey],
    message: &[u8; 32],
    signature: &BlsSignature,
) -> bool {
    if pubkeys.is_empty() {
        return false;
    }
    let Ok(signature) = Signature::sig_validate(signature, false) else {
        return false;
    };
    let mut keys = Vec::with_capacity(pubkeys.len());
    for bytes in pubkeys {
        match PublicKey::key_validate(bytes) {
            Ok(key) => keys.push(key),
            Err(_) => return false,
        }
    }
    let refs: Vec<&PublicKey> = keys.iter().collect();
    signature.fast_aggregate_verify(false, message, DST, &refs) == BLST_ERROR::BLST_SUCCESS
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use blst::min_pk::{AggregateSignature, SecretKey};

    fn key(seed: u8) -> SecretKey {
        SecretKey::key_gen(&[seed; 32], &[]).unwrap()
    }

    #[test]
    fn aggregate_of_three_verifies_and_a_missing_signer_fails() {
        let keys: Vec<SecretKey> = (1..=3).map(key).collect();
        let message = [7u8; 32];
        let sigs: Vec<Signature> = keys.iter().map(|k| k.sign(&message, DST, &[])).collect();
        let refs: Vec<&Signature> = sigs.iter().collect();
        let aggregate = AggregateSignature::aggregate(&refs, true)
            .unwrap()
            .to_signature();
        let pubkeys: Vec<BlsPubkey> = keys.iter().map(|k| k.sk_to_pk().to_bytes()).collect();
        assert!(fast_aggregate_verify(
            &pubkeys,
            &message,
            &aggregate.to_bytes()
        ));
        assert!(!fast_aggregate_verify(
            &pubkeys[..2],
            &message,
            &aggregate.to_bytes()
        ));
        assert!(!fast_aggregate_verify(
            &pubkeys,
            &[8u8; 32],
            &aggregate.to_bytes()
        ));
        assert!(!fast_aggregate_verify(&[], &message, &aggregate.to_bytes()));
    }
}
