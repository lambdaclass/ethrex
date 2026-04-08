//! 0x69 — secp256k1SignatureRecover
//!
//! Verifies a Tendermint-style secp256k1 signature and returns the signer's
//! Tendermint address (first 20 bytes of the RIPEMD-160(SHA-256(compressed_pubkey))).
//!
//! This is distinct from EVM's ECRECOVER (0x01): the caller provides the
//! compressed public key explicitly.  The precompile verifies the (r||s)
//! signature against the supplied message hash using the public key, and on
//! success derives the Tendermint address.  No recovery ID is needed.
//!
//! Input layout (129 bytes):
//! | compressed_pubkey (33) | signature (64) | msg_hash (32) |
//!
//! Output on success: 20-byte Tendermint address = RIPEMD-160(SHA-256(pubkey)).
//!
//! Gas: 3 000  (`params.EcrecoverGas` — reuses the ecrecover gas constant)
//!
//! Reference: BSC `core/vm/contracts_lightclient.go`, `secp256k1SignatureRecover`.
//! Signature semantics: BNB-chain Tendermint `PubKeySecp256k1::VerifyBytesWithMsgHash`
//! (verifies R||S directly against the hash, lower-S form required).

use ripemd::{Digest as RipemdDigest, Ripemd160};
use sha2::{Digest as Sha2Digest, Sha256};

use super::PrecompileError;

/// Gas cost for secp256k1SignatureRecover.  Matches `params.EcrecoverGas`.
pub const SECP256K1_RECOVER_GAS: u64 = 3_000;

/// Expected input length: 33 (pubkey) + 64 (sig) + 32 (msg_hash).
pub const INPUT_LENGTH: usize = 129;

/// Byte offset where the signature begins in the input.
const SIG_OFFSET: usize = 33;
/// Byte offset where the message hash begins in the input.
const HASH_OFFSET: usize = 33 + 64;

/// Run the secp256k1SignatureRecover precompile.
///
/// 1. Parses the 33-byte compressed secp256k1 public key from the input.
/// 2. Verifies the 64-byte compact (r||s) signature against the 32-byte
///    message hash using the supplied public key.  The signature must be in
///    lower-S form (malleable high-S signatures are rejected).
/// 3. Computes and returns the 20-byte Tendermint address:
///    RIPEMD-160(SHA-256(compressed_pubkey)).
pub fn run(input: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    if gas_limit < SECP256K1_RECOVER_GAS {
        return Err(PrecompileError::NotEnoughGas);
    }
    if input.len() != INPUT_LENGTH {
        return Err(PrecompileError::InvalidInput);
    }

    let pubkey_bytes = &input[..SIG_OFFSET];
    let sig_bytes = &input[SIG_OFFSET..HASH_OFFSET];
    let msg_hash = &input[HASH_OFFSET..];

    // Parse the compressed public key (33 bytes: 0x02/0x03 prefix + 32-byte x).
    let pubkey = secp256k1::PublicKey::from_slice(pubkey_bytes)
        .map_err(|_| PrecompileError::InvalidInput)?;

    // Parse the compact (r||s) signature.
    let sig = secp256k1::ecdsa::Signature::from_compact(sig_bytes)
        .map_err(|_| PrecompileError::InvalidInput)?;

    // Reject high-S signatures (matches Tendermint's lower-S requirement).
    // normalize_s() normalises in-place; if the compact form changed, S was
    // above the half-order and the signature is malleable — reject it.
    {
        let before = sig.serialize_compact();
        let mut check_sig = sig;
        check_sig.normalize_s();
        let after = check_sig.serialize_compact();
        if before != after {
            return Err(PrecompileError::InvalidInput);
        }
    }

    // Verify the signature against the raw message hash.
    // `secp256k1::Message::from_digest` treats the 32 bytes as a pre-computed
    // hash, matching Tendermint's `VerifyBytesWithMsgHash` behaviour.
    let msg_array: [u8; 32] = msg_hash
        .try_into()
        .map_err(|_| PrecompileError::InvalidInput)?;
    let message = secp256k1::Message::from_digest(msg_array);

    sig.verify(&message, &pubkey)
        .map_err(|_| PrecompileError::ExecutionReverted)?;

    // Derive the Tendermint address: RIPEMD-160(SHA-256(compressed_pubkey)).
    let sha256_hash = <Sha256 as Sha2Digest>::digest(pubkey_bytes);
    let ripemd_hash = <Ripemd160 as RipemdDigest>::digest(sha256_hash);

    Ok((SECP256K1_RECOVER_GAS, ripemd_hash.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_rejects_insufficient_gas() {
        let input = [0u8; INPUT_LENGTH];
        let err = run(&input, SECP256K1_RECOVER_GAS - 1).unwrap_err();
        assert_eq!(err, PrecompileError::NotEnoughGas);
    }

    #[test]
    fn run_rejects_wrong_length() {
        // Too short
        let err = run(&[0u8; 128], SECP256K1_RECOVER_GAS).unwrap_err();
        assert_eq!(err, PrecompileError::InvalidInput);

        // Too long
        let err = run(&[0u8; 130], SECP256K1_RECOVER_GAS).unwrap_err();
        assert_eq!(err, PrecompileError::InvalidInput);
    }

    #[test]
    fn run_rejects_invalid_pubkey() {
        // All-zero compressed pubkey is invalid on secp256k1.
        let input = [0u8; INPUT_LENGTH];
        let err = run(&input, SECP256K1_RECOVER_GAS).unwrap_err();
        assert_eq!(err, PrecompileError::InvalidInput);
    }

    /// Round-trip test: sign a message with a known key and verify the precompile
    /// returns the expected Tendermint address.
    #[test]
    fn run_valid_signature_returns_tendermint_address() {
        use ripemd::{Digest as _, Ripemd160};
        use secp256k1::{Message, Secp256k1, SecretKey};
        use sha2::{Digest as _, Sha256};

        let secp = Secp256k1::new();

        // Generate a deterministic secret key for testing.
        let sk_bytes = [0x42u8; 32];
        let sk = SecretKey::from_slice(&sk_bytes).unwrap();
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
        let pk_compressed = pk.serialize(); // 33 bytes

        // Compute a test message hash.
        let msg_hash_bytes = Sha256::digest(b"test message double sign");
        let msg_hash_arr: [u8; 32] = msg_hash_bytes.into();
        let message = Message::from_digest(msg_hash_arr);

        // Sign with lower-S enforced (ECDSA sign_ecdsa produces normalized sigs).
        let sig = secp.sign_ecdsa(&message, &sk);
        let sig_compact = sig.serialize_compact(); // 64 bytes

        // Build the 129-byte input.
        let mut input = [0u8; INPUT_LENGTH];
        input[..33].copy_from_slice(&pk_compressed);
        input[33..97].copy_from_slice(&sig_compact);
        input[97..].copy_from_slice(&msg_hash_arr);

        let (gas_used, output) = run(&input, SECP256K1_RECOVER_GAS).unwrap();
        assert_eq!(gas_used, SECP256K1_RECOVER_GAS);
        assert_eq!(output.len(), 20);

        // Verify the address matches RIPEMD160(SHA256(pk_compressed)).
        let sha = Sha256::digest(&pk_compressed);
        let ripemd = Ripemd160::digest(&sha);
        assert_eq!(output, ripemd.as_slice());
    }
}
