use bytes::Bytes;
use ethereum_types::Address;
use ethrex_common::H256;
use ethrex_common::types::BlockHeader;
use ethrex_crypto::{Crypto, CryptoError, NativeCrypto, keccak::keccak_hash};
use ethrex_rlp::structs::Encoder;

use super::extra_data::{
    EXTRA_SEAL_LENGTH, EXTRA_VANITY_LENGTH, ExtraDataError, extract_signature,
};

/// Errors from seal hash computation or signer recovery.
#[derive(Debug, thiserror::Error)]
pub enum SealError {
    #[error(
        "extra data too short for signature extraction: need at least {EXTRA_SEAL_LENGTH} bytes, got {0}"
    )]
    ExtraDataTooShort(usize),
    #[error(
        "extra data too short for seal hash: need at least {EXTRA_VANITY_LENGTH} bytes, got {0}"
    )]
    ExtraDataTooShortForVanity(usize),
    #[error("crypto error during signer recovery: {0}")]
    Crypto(#[from] CryptoError),
    #[error("extra data parse error: {0}")]
    ExtraData(#[from] ExtraDataError),
}

/// Compute the Parlia seal hash of a BSC block header.
///
/// BSC Parlia uses a chain-ID-prefixed RLP encoding that differs from standard Clique.
/// The Extra field in the hash is truncated to the first `EXTRA_VANITY_LENGTH` (32) bytes
/// only — no validator list, no attestation, no seal — matching BSC's
/// `encodeSigHeaderWithoutVoteAttestation`.
///
/// Reference: BSC `consensus/parlia/parlia.go`, `encodeSigHeaderWithoutVoteAttestation`
/// at line 1916.
pub fn seal_hash(header: &BlockHeader, chain_id: u64) -> Result<H256, SealError> {
    let extra = &header.extra_data;
    if extra.len() < EXTRA_VANITY_LENGTH {
        return Err(SealError::ExtraDataTooShortForVanity(extra.len()));
    }

    // Only the first 32 bytes (vanity) of extra are included in the seal hash.
    let vanity_only = Bytes::copy_from_slice(&extra[..EXTRA_VANITY_LENGTH]);

    let mut buf = Vec::with_capacity(1024);

    // chain_id is encoded as a u64 (matching Go's big.Int encoding for small values).
    let mut encoder = Encoder::new(&mut buf)
        .encode_field(&chain_id)
        .encode_field(&header.parent_hash)
        .encode_field(&header.ommers_hash)
        .encode_field(&header.coinbase)
        .encode_field(&header.state_root)
        .encode_field(&header.transactions_root)
        .encode_field(&header.receipts_root)
        .encode_field(&header.logs_bloom)
        .encode_field(&header.difficulty)
        .encode_field(&header.number)
        .encode_field(&header.gas_limit)
        .encode_field(&header.gas_used)
        .encode_field(&header.timestamp)
        .encode_field(&vanity_only)
        .encode_field(&header.prev_randao)
        .encode_field(&header.nonce.to_be_bytes());

    encoder = encoder.encode_optional_field(&header.base_fee_per_gas);
    encoder = encoder.encode_optional_field(&header.withdrawals_root);
    encoder = encoder.encode_optional_field(&header.blob_gas_used);
    encoder = encoder.encode_optional_field(&header.excess_blob_gas);
    encoder = encoder.encode_optional_field(&header.parent_beacon_block_root);
    encoder = encoder.encode_optional_field(&header.requests_hash);

    encoder.finish();

    Ok(H256(keccak_hash(&buf)))
}

/// Compute the block-style seal hash used by `core/types.SealHash` in bsc-geth.
///
/// This differs from [`seal_hash`] (the Parlia variant): it includes the full
/// `extra` minus the trailing 65-byte signature, and conditionally appends
/// Cancun+ fields when `parent_beacon_block_root` is set. Used by the
/// `verifyDoubleSignEvidence` precompile (0x68), which calls into
/// `types.SealHash` rather than the Parlia engine variant.
///
/// Reference: bsc-geth `core/types/block.go` `EncodeSigHeader`.
pub fn block_seal_hash(header: &BlockHeader, chain_id: u64) -> Result<H256, SealError> {
    let extra = &header.extra_data;
    if extra.len() < EXTRA_SEAL_LENGTH {
        return Err(SealError::ExtraDataTooShort(extra.len()));
    }

    // Extra minus the trailing 65-byte signature.
    let extra_no_seal = Bytes::copy_from_slice(&extra[..extra.len() - EXTRA_SEAL_LENGTH]);

    let mut buf = Vec::with_capacity(1024);
    let mut encoder = Encoder::new(&mut buf)
        .encode_field(&chain_id)
        .encode_field(&header.parent_hash)
        .encode_field(&header.ommers_hash)
        .encode_field(&header.coinbase)
        .encode_field(&header.state_root)
        .encode_field(&header.transactions_root)
        .encode_field(&header.receipts_root)
        .encode_field(&header.logs_bloom)
        .encode_field(&header.difficulty)
        .encode_field(&header.number)
        .encode_field(&header.gas_limit)
        .encode_field(&header.gas_used)
        .encode_field(&header.timestamp)
        .encode_field(&extra_no_seal)
        .encode_field(&header.prev_randao)
        .encode_field(&header.nonce.to_be_bytes());

    // Cancun+ fields are only appended when ParentBeaconRoot is set on the
    // header, matching the Go `if header.ParentBeaconRoot != nil` branch.
    // BSC mainnet headers don't carry a beacon root, so this branch is a no-op
    // for them; included here for parity with the reference.
    if let Some(parent_beacon_root) = header.parent_beacon_block_root {
        let base_fee = header.base_fee_per_gas.unwrap_or(0);
        let withdrawals_root = header.withdrawals_root.unwrap_or_default();
        let blob_gas_used = header.blob_gas_used.unwrap_or(0);
        let excess_blob_gas = header.excess_blob_gas.unwrap_or(0);
        encoder = encoder
            .encode_field(&base_fee)
            .encode_field(&withdrawals_root)
            .encode_field(&blob_gas_used)
            .encode_field(&excess_blob_gas)
            .encode_field(&parent_beacon_root);
        if let Some(requests_hash) = header.requests_hash {
            encoder = encoder.encode_field(&requests_hash);
        }
    }

    encoder.finish();
    Ok(H256(keccak_hash(&buf)))
}

/// Recover the signer address from a sealed BSC block header.
///
/// Computes the seal hash (with chain ID), extracts the 65-byte ECDSA signature
/// from the end of the Extra field, and uses secp256k1 ecrecover to derive the
/// signer's Ethereum address.
pub fn recover_signer(header: &BlockHeader, chain_id: u64) -> Result<Address, SealError> {
    let extra = &header.extra_data;
    if extra.len() < EXTRA_SEAL_LENGTH {
        return Err(SealError::ExtraDataTooShort(extra.len()));
    }

    let hash = seal_hash(header, chain_id)?;
    let sig_bytes = extract_signature(extra)?;

    let crypto = NativeCrypto;
    let address = crypto.recover_signer(&sig_bytes, hash.as_fixed_bytes())?;
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::types::BlockHeader;

    fn minimal_header() -> BlockHeader {
        BlockHeader {
            extra_data: Bytes::from(vec![0u8; EXTRA_VANITY_LENGTH + EXTRA_SEAL_LENGTH]),
            ..Default::default()
        }
    }

    #[test]
    fn seal_hash_returns_ok_with_valid_extra() {
        let header = minimal_header();
        let result = seal_hash(&header, 56);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    #[test]
    fn seal_hash_fails_extra_too_short() {
        let mut header = minimal_header();
        header.extra_data = Bytes::from(vec![0u8; 10]);
        let result = seal_hash(&header, 56);
        assert!(
            matches!(result, Err(SealError::ExtraDataTooShortForVanity(10))),
            "unexpected result: {:?}",
            result
        );
    }

    #[test]
    fn seal_hash_differs_by_chain_id() {
        let header = minimal_header();
        let h1 = seal_hash(&header, 56).unwrap();
        let h2 = seal_hash(&header, 97).unwrap();
        assert_ne!(h1, h2, "seal hashes should differ for different chain IDs");
    }

    #[test]
    fn recover_signer_fails_extra_too_short() {
        let mut header = minimal_header();
        header.extra_data = Bytes::from(vec![0u8; 10]);
        let result = recover_signer(&header, 56);
        assert!(
            matches!(result, Err(SealError::ExtraDataTooShort(10))),
            "unexpected result: {:?}",
            result
        );
    }
}
