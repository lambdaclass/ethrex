use std::time::{SystemTime, UNIX_EPOCH};

use ethrex_common::U256;
use ethrex_common::constants::DEFAULT_OMMERS_HASH;
use ethrex_common::types::BlockHeader;

use crate::consensus::extra_data::{EXTRA_SEAL_LENGTH, EXTRA_VANITY_LENGTH};
use crate::parlia_config::ParliaConfig;

/// Errors arising from BSC Parlia header structural validation.
#[derive(Debug, thiserror::Error)]
pub enum ParliaValidationError {
    #[error("nonce must be zero for BSC PoA headers")]
    NonceNotZero,
    #[error("uncle hash must be the empty uncle hash (PoA chains have no uncles)")]
    InvalidUncleHash,
    #[error("difficulty must be 1 (no-turn) or 2 (in-turn), got {0}")]
    InvalidDifficulty(U256),
    #[error("extra data too short: need at least {min} bytes (vanity + seal), got {actual}")]
    ExtraDataTooShort { min: usize, actual: usize },
    #[error("epoch block {block_number} has invalid or missing validator data in extra field")]
    MissingEpochValidators { block_number: u64 },
    #[error("block timestamp {0} is in the future")]
    FutureBlock(u64),
    #[error("block number is not one greater than parent")]
    BlockNumberNotOneGreater,
    #[error("block timestamp must be greater than parent timestamp")]
    TimestampNotGreaterThanParent,
    #[error(
        "lorentz milli-timestamp is inconsistent with header.time: milli_ts={milli_ts}, time={time}"
    )]
    InvalidLorentzMilliTimestamp { milli_ts: u64, time: u64 },
}

const DIFF_IN_TURN: u64 = 2;
const DIFF_NO_TURN: u64 = 1;

/// Validate the structural properties of a BSC (Parlia) block header.
///
/// Checks performed:
/// - Block number is one greater than parent
/// - Timestamp is greater than parent (and not in the future)
/// - Nonce is zero (BSC PoA)
/// - Uncle hash is the empty uncle hash
/// - Difficulty is 1 (no-turn) or 2 (in-turn)
/// - Extra data has at least vanity + seal bytes
/// - Epoch blocks contain valid validator data in the extra field
/// - For post-Lorentz blocks, the milli-timestamp encoded in `mix_digest` is
///   consistent with `header.time`
pub fn validate_parlia_header(
    header: &BlockHeader,
    parent: &BlockHeader,
    config: &ParliaConfig,
    is_lorentz: bool,
) -> Result<(), ParliaValidationError> {
    // Block number must be parent + 1
    if header.number != parent.number + 1 {
        return Err(ParliaValidationError::BlockNumberNotOneGreater);
    }

    // Timestamp must be strictly greater than parent
    if header.timestamp <= parent.timestamp {
        return Err(ParliaValidationError::TimestampNotGreaterThanParent);
    }

    // Block must not be too far in the future.
    // BSC reference allows up to 15 seconds of clock skew
    // (`allowedFutureBlockTimeSeconds = 15` in `consensus/parlia/parlia.go`).
    const ALLOWED_FUTURE_BLOCK_SECONDS: u64 = 15;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if header.timestamp > now + ALLOWED_FUTURE_BLOCK_SECONDS {
        return Err(ParliaValidationError::FutureBlock(header.timestamp));
    }

    // Nonce must be zero for BSC PoA
    if header.nonce != 0 {
        return Err(ParliaValidationError::NonceNotZero);
    }

    // Uncle hash must be the empty uncle hash (BSC doesn't use uncles)
    if header.ommers_hash != *DEFAULT_OMMERS_HASH {
        return Err(ParliaValidationError::InvalidUncleHash);
    }

    // Difficulty must be 1 (no-turn) or 2 (in-turn) for non-genesis blocks
    if header.number > 0 {
        let diff = header.difficulty.as_u64();
        if header.difficulty != U256::from(DIFF_IN_TURN)
            && header.difficulty != U256::from(DIFF_NO_TURN)
        {
            return Err(ParliaValidationError::InvalidDifficulty(header.difficulty));
        }
        let _ = diff; // consumed for clarity
    }

    // Extra data must have at least vanity (32) + seal (65) bytes
    let min_extra = EXTRA_VANITY_LENGTH + EXTRA_SEAL_LENGTH;
    if header.extra_data.len() < min_extra {
        return Err(ParliaValidationError::ExtraDataTooShort {
            min: min_extra,
            actual: header.extra_data.len(),
        });
    }

    // At epoch blocks, extra must contain a non-empty validator list.
    // A validator count of 0 or a buffer too short to hold any entries is invalid.
    if config.is_epoch_start(header.number, header.timestamp) {
        let extra = &header.extra_data;
        // The byte immediately after the vanity is the validator count (post-Luban format).
        // We require at least vanity + count_byte + seal.
        let count_offset = EXTRA_VANITY_LENGTH;
        if extra.len() <= count_offset + EXTRA_SEAL_LENGTH {
            return Err(ParliaValidationError::MissingEpochValidators {
                block_number: header.number,
            });
        }
        let validator_count = extra[count_offset] as usize;
        if validator_count == 0 {
            return Err(ParliaValidationError::MissingEpochValidators {
                block_number: header.number,
            });
        }
    }

    // For post-Lorentz blocks, the milli-timestamp is:
    //   milli_ts = header.time * 1000 + uint256(mix_digest)
    // The high-order portion must equal header.time when divided by 1000.
    if is_lorentz {
        // mix_digest encodes the millisecond sub-second offset (0-999) as a big-endian u256.
        let millis_part = {
            let bytes = header.prev_randao.as_bytes();
            // Take the last 8 bytes as little-endian u64 (the value is expected to be 0-999)
            let mut arr = [0u8; 8];
            arr.copy_from_slice(&bytes[24..32]);
            u64::from_be_bytes(arr)
        };
        let milli_ts = header
            .timestamp
            .saturating_mul(1000)
            .saturating_add(millis_part);
        if milli_ts / 1000 != header.timestamp {
            return Err(ParliaValidationError::InvalidLorentzMilliTimestamp {
                milli_ts,
                time: header.timestamp,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::U256;
    use ethrex_common::constants::DEFAULT_OMMERS_HASH;
    use ethrex_common::types::BlockHeader;

    fn valid_header(number: u64, timestamp: u64) -> BlockHeader {
        BlockHeader {
            number,
            timestamp,
            ommers_hash: *DEFAULT_OMMERS_HASH,
            difficulty: U256::from(2u64),
            nonce: 0,
            extra_data: Bytes::from(vec![0u8; EXTRA_VANITY_LENGTH + EXTRA_SEAL_LENGTH]),
            ..Default::default()
        }
    }

    fn valid_parent(number: u64, timestamp: u64) -> BlockHeader {
        let mut h = valid_header(number, timestamp);
        h.extra_data = Bytes::from(vec![0u8; EXTRA_VANITY_LENGTH + EXTRA_SEAL_LENGTH]);
        h
    }

    fn config() -> ParliaConfig {
        ParliaConfig::default()
    }

    #[test]
    fn valid_non_epoch_header() {
        let parent = valid_parent(1, 1_000_000_000);
        // Use a timestamp well in the past so the future check passes.
        let mut header = valid_header(2, 1_000_000_003);
        header.parent_hash = parent.hash();
        assert!(validate_parlia_header(&header, &parent, &config(), false).is_ok());
    }

    #[test]
    fn rejects_nonzero_nonce() {
        let parent = valid_parent(1, 1_000_000_000);
        let mut header = valid_header(2, 1_000_000_003);
        header.nonce = 1;
        let err = validate_parlia_header(&header, &parent, &config(), false).unwrap_err();
        assert!(matches!(err, ParliaValidationError::NonceNotZero));
    }

    #[test]
    fn rejects_wrong_difficulty() {
        let parent = valid_parent(1, 1_000_000_000);
        let mut header = valid_header(2, 1_000_000_003);
        header.difficulty = U256::from(5u64);
        let err = validate_parlia_header(&header, &parent, &config(), false).unwrap_err();
        assert!(matches!(err, ParliaValidationError::InvalidDifficulty(_)));
    }

    #[test]
    fn rejects_extra_data_too_short() {
        let parent = valid_parent(1, 1_000_000_000);
        let mut header = valid_header(2, 1_000_000_003);
        header.extra_data = Bytes::from(vec![0u8; 10]);
        let err = validate_parlia_header(&header, &parent, &config(), false).unwrap_err();
        assert!(matches!(
            err,
            ParliaValidationError::ExtraDataTooShort { .. }
        ));
    }

    #[test]
    fn rejects_wrong_uncle_hash() {
        let parent = valid_parent(1, 1_000_000_000);
        let mut header = valid_header(2, 1_000_000_003);
        header.ommers_hash = ethrex_common::H256::zero();
        let err = validate_parlia_header(&header, &parent, &config(), false).unwrap_err();
        assert!(matches!(err, ParliaValidationError::InvalidUncleHash));
    }

    #[test]
    fn rejects_timestamp_not_greater_than_parent() {
        let parent = valid_parent(1, 1_000_000_010);
        let header = valid_header(2, 1_000_000_010);
        let err = validate_parlia_header(&header, &parent, &config(), false).unwrap_err();
        assert!(matches!(
            err,
            ParliaValidationError::TimestampNotGreaterThanParent
        ));
    }

    #[test]
    fn rejects_block_number_not_one_greater() {
        let parent = valid_parent(1, 1_000_000_000);
        let header = valid_header(5, 1_000_000_003);
        let err = validate_parlia_header(&header, &parent, &config(), false).unwrap_err();
        assert!(matches!(
            err,
            ParliaValidationError::BlockNumberNotOneGreater
        ));
    }
}
