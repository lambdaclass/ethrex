//! 0x68 — verifyDoubleSignEvidence
//!
//! Verifies RLP-encoded double-sign evidence for the BSC slashing mechanism.
//! Given two different block headers signed by the same validator at the same
//! height, recovers the signer address and returns it together with the block
//! height.
//!
//! Input: RLP-encoded `DoubleSignEvidence { chain_id (U256), header_bytes_1 (bytes), header_bytes_2 (bytes) }`.
//!
//! Output on success (52 bytes):
//! | signer_address (20) | evidence_height (32) |
//!
//! Gas: 10 000  (`params.DoubleSignEvidenceVerifyGas`)
//!
//! Reference: BSC `core/vm/contracts.go`, `verifyDoubleSignEvidence`.

use bytes::Bytes;
use ethrex_common::{U256, types::BlockHeader};
use ethrex_rlp::{decode::RLPDecode, error::RLPDecodeError, structs::Decoder};

use super::PrecompileError;
use crate::consensus::extra_data::EXTRA_SEAL_LENGTH;
use crate::consensus::seal::seal_hash;

/// Gas cost for verifyDoubleSignEvidence.  Matches `params.DoubleSignEvidenceVerifyGas`.
pub const DOUBLE_SIGN_EVIDENCE_GAS: u64 = 10_000;

/// RLP-decodable representation of the double-sign evidence input.
struct DoubleSignEvidence {
    chain_id: U256,
    header_bytes1: Bytes,
    header_bytes2: Bytes,
}

impl RLPDecode for DoubleSignEvidence {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (chain_id, decoder) = decoder.decode_field("chain_id")?;
        let (header_bytes1, decoder) = decoder.decode_field("header_bytes1")?;
        let (header_bytes2, decoder) = decoder.decode_field("header_bytes2")?;
        let rest = decoder.finish()?;
        Ok((
            DoubleSignEvidence {
                chain_id,
                header_bytes1,
                header_bytes2,
            },
            rest,
        ))
    }
}

/// Run the verifyDoubleSignEvidence precompile.
///
/// Decodes two BSC block headers from the RLP-encoded evidence, checks that
/// both were signed by the same validator at the same block height but with
/// different signatures (proving equivocation), and returns the signer address
/// together with the block height.
///
/// Output layout (52 bytes):
/// | signer_address (20 bytes) | evidence_height (32 bytes, big-endian) |
pub fn run(input: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    if gas_limit < DOUBLE_SIGN_EVIDENCE_GAS {
        return Err(PrecompileError::NotEnoughGas);
    }
    if input.is_empty() {
        return Err(PrecompileError::InvalidInput);
    }

    // Step 1: RLP-decode the outer DoubleSignEvidence envelope.
    let evidence =
        DoubleSignEvidence::decode(input).map_err(|_| PrecompileError::ExecutionReverted)?;

    let chain_id: u64 = evidence
        .chain_id
        .try_into()
        .map_err(|_| PrecompileError::InvalidInput)?;

    // Step 2: RLP-decode each header from its byte payload.
    let header1 = BlockHeader::decode(&evidence.header_bytes1)
        .map_err(|_| PrecompileError::ExecutionReverted)?;
    let header2 = BlockHeader::decode(&evidence.header_bytes2)
        .map_err(|_| PrecompileError::ExecutionReverted)?;

    // Step 3: Basic validity checks (mirrors BSC Go source).
    // Block numbers must be identical.
    if header1.number != header2.number {
        return Err(PrecompileError::InvalidInput);
    }
    // Parent hashes must be identical.
    if header1.parent_hash != header2.parent_hash {
        return Err(PrecompileError::InvalidInput);
    }
    // Both extra fields must be long enough to hold a 65-byte seal.
    if header1.extra_data.len() < EXTRA_SEAL_LENGTH || header2.extra_data.len() < EXTRA_SEAL_LENGTH
    {
        return Err(PrecompileError::InvalidInput);
    }
    // Seals must differ (otherwise it is not evidence of equivocation).
    let sig1 = &header1.extra_data[header1.extra_data.len() - EXTRA_SEAL_LENGTH..];
    let sig2 = &header2.extra_data[header2.extra_data.len() - EXTRA_SEAL_LENGTH..];
    if sig1 == sig2 {
        return Err(PrecompileError::InvalidInput);
    }

    // Step 4: Compute the seal hash for each header and verify they differ.
    let hash1 = seal_hash(&header1, chain_id).map_err(|_| PrecompileError::InvalidInput)?;
    let hash2 = seal_hash(&header2, chain_id).map_err(|_| PrecompileError::InvalidInput)?;
    if hash1 == hash2 {
        return Err(PrecompileError::InvalidInput);
    }

    // Step 5: Recover uncompressed public keys from each header seal.
    // We use secp256k1 directly (same library as ethrex-crypto's NativeCrypto) to
    // get the raw uncompressed public key bytes for Ethereum address derivation.
    let sig1_bytes: [u8; 65] = sig1.try_into().expect("sig1 is exactly 65 bytes");
    let sig2_bytes: [u8; 65] = sig2.try_into().expect("sig2 is exactly 65 bytes");

    let pubkey1 = recover_pubkey_uncompressed(&sig1_bytes, hash1.as_fixed_bytes())
        .map_err(|_| PrecompileError::ExecutionReverted)?;
    let pubkey2 = recover_pubkey_uncompressed(&sig2_bytes, hash2.as_fixed_bytes())
        .map_err(|_| PrecompileError::ExecutionReverted)?;

    // Step 6: Both uncompressed public keys must be the same.
    if pubkey1 != pubkey2 {
        return Err(PrecompileError::InvalidInput);
    }

    // Step 7: Build the 52-byte output.
    // Signer address = keccak256(pubkey[1..])[12..] (standard Ethereum address).
    // Evidence height as big-endian U256 (zero-padded).
    use ethrex_crypto::keccak::keccak_hash;
    let addr_hash = keccak_hash(&pubkey1[1..]);
    let signer_addr = &addr_hash[12..]; // last 20 bytes of keccak256

    let mut output = vec![0u8; 52];
    output[..20].copy_from_slice(signer_addr);

    // Encode the block number as a 32-byte big-endian value.
    let mut height_bytes = [0u8; 32];
    let number_be = header1.number.to_be_bytes(); // u64 → 8 bytes
    height_bytes[24..].copy_from_slice(&number_be);
    output[20..].copy_from_slice(&height_bytes);

    Ok((DOUBLE_SIGN_EVIDENCE_GAS, output))
}

/// Recover the 65-byte uncompressed public key from a 65-byte compact signature
/// (r || s || v) and a 32-byte message hash.
///
/// Uses the `secp256k1` C library directly so we can obtain the uncompressed
/// key bytes needed for Ethereum address derivation.
fn recover_pubkey_uncompressed(
    sig: &[u8; 65],
    msg: &[u8; 32],
) -> Result<[u8; 65], secp256k1::Error> {
    let recid = secp256k1::ecdsa::RecoveryId::try_from(sig[64] as i32)?;
    let recoverable = secp256k1::ecdsa::RecoverableSignature::from_compact(&sig[..64], recid)?;
    let message = secp256k1::Message::from_digest(*msg);
    let pubkey = recoverable.recover(&message)?;
    Ok(pubkey.serialize_uncompressed())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_rejects_empty_input() {
        let err = run(&[], 10_000).unwrap_err();
        assert_eq!(err, PrecompileError::InvalidInput);
    }

    #[test]
    fn run_rejects_insufficient_gas() {
        let err = run(&[0u8; 10], 9_999).unwrap_err();
        assert_eq!(err, PrecompileError::NotEnoughGas);
    }

    #[test]
    fn run_rejects_malformed_rlp() {
        // All-zero bytes are not valid RLP for a DoubleSignEvidence list.
        let err = run(&[0u8; 64], 10_000).unwrap_err();
        assert_eq!(err, PrecompileError::ExecutionReverted);
    }
}
