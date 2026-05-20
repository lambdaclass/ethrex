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
use crate::consensus::seal::block_seal_hash;

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
    Ok((DOUBLE_SIGN_EVIDENCE_GAS, run_inner(input).unwrap_or_default()))
}

/// Compute the precompile output. Returns `None` for any input-validation or
/// recovery failure — callers should treat that as the EVM "predictable
/// failure" path: precompile consumes the required gas, returns empty output.
/// Mirrors bsc-geth's pattern (ECRECOVER, modexp, etc.) where bad input does
/// NOT burn the caller's forwarded gas.
fn run_inner(input: &[u8]) -> Option<Vec<u8>> {
    if input.is_empty() {
        return None;
    }

    // Step 1: RLP-decode the outer DoubleSignEvidence envelope.
    let evidence = DoubleSignEvidence::decode(input).ok()?;
    let chain_id: u64 = evidence.chain_id.try_into().ok()?;

    // Step 2: RLP-decode each header from its byte payload.
    let header1 = BlockHeader::decode(&evidence.header_bytes1).ok()?;
    let header2 = BlockHeader::decode(&evidence.header_bytes2).ok()?;

    // Step 3: Basic validity checks (mirrors BSC Go source).
    if header1.number != header2.number {
        return None;
    }
    if header1.parent_hash != header2.parent_hash {
        return None;
    }
    if header1.extra_data.len() < EXTRA_SEAL_LENGTH || header2.extra_data.len() < EXTRA_SEAL_LENGTH
    {
        return None;
    }
    let sig1 = &header1.extra_data[header1.extra_data.len() - EXTRA_SEAL_LENGTH..];
    let sig2 = &header2.extra_data[header2.extra_data.len() - EXTRA_SEAL_LENGTH..];
    if sig1 == sig2 {
        return None;
    }

    // Step 4: Compute the seal hash for each header and verify they differ.
    // Uses `block_seal_hash` (mirrors bsc-geth's `core/types.SealHash`), NOT
    // the Parlia `seal_hash` — the precompile reference calls
    // `types.SealHash(header, chainId)` which encodes `extra[:-extraSeal]`,
    // not just the 32-byte vanity prefix.
    let hash1 = block_seal_hash(&header1, chain_id).ok()?;
    let hash2 = block_seal_hash(&header2, chain_id).ok()?;
    if hash1 == hash2 {
        return None;
    }

    // Step 5: Recover uncompressed public keys from each header seal.
    let sig1_bytes: [u8; 65] = sig1.try_into().ok()?;
    let sig2_bytes: [u8; 65] = sig2.try_into().ok()?;
    let pubkey1 = recover_pubkey_uncompressed(&sig1_bytes, hash1.as_fixed_bytes()).ok()?;
    let pubkey2 = recover_pubkey_uncompressed(&sig2_bytes, hash2.as_fixed_bytes()).ok()?;

    // Step 6: Both uncompressed public keys must be the same.
    if pubkey1 != pubkey2 {
        return None;
    }

    // Step 7: Build the 52-byte output.
    use ethrex_crypto::keccak::keccak_hash;
    let addr_hash = keccak_hash(&pubkey1[1..]);
    let signer_addr = &addr_hash[12..];

    let mut output = vec![0u8; 52];
    output[..20].copy_from_slice(signer_addr);

    let mut height_bytes = [0u8; 32];
    let number_be = header1.number.to_be_bytes();
    height_bytes[24..].copy_from_slice(&number_be);
    output[20..].copy_from_slice(&height_bytes);

    Some(output)
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
