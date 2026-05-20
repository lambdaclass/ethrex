use ethereum_types::Address;
use ethrex_common::H256;
use ethrex_rlp::{decode::RLPDecode, error::RLPDecodeError, structs::Decoder};

/// Fixed number of extra-data prefix bytes reserved for signer vanity.
pub const EXTRA_VANITY_LENGTH: usize = 32;
/// Fixed number of extra-data suffix bytes reserved for signer seal (signature).
pub const EXTRA_SEAL_LENGTH: usize = 65;
/// Number of bytes used to encode the validator count at epoch blocks.
const VALIDATOR_NUMBER_SIZE: usize = 1;
/// Number of bytes per validator entry: 20-byte address + 48-byte BLS pubkey.
const VALIDATOR_BYTES_LENGTH: usize = 20 + 48;
/// Number of bytes used to encode the turn length at epoch blocks (post-Bohr).
const TURN_LENGTH_SIZE: usize = 1;

/// Errors returned by extra-data parsing functions.
#[derive(Debug, thiserror::Error)]
pub enum ExtraDataError {
    #[error("extra data too short: need at least {0} bytes, got {1}")]
    TooShort(usize, usize),
    #[error("invalid validator count: buffer too small for {0} validators")]
    InvalidValidatorCount(usize),
    #[error("turn length byte missing in epoch extra data")]
    MissingTurnLength,
    #[error("RLP decode error: {0}")]
    Rlp(#[from] RLPDecodeError),
}

/// Strip the 65-byte signature from the end of extra_data.
///
/// If extra_data is shorter than EXTRA_SEAL_LENGTH, returns an empty slice.
pub fn strip_signature(extra: &[u8]) -> &[u8] {
    if extra.len() < EXTRA_SEAL_LENGTH {
        return &[];
    }
    &extra[..extra.len() - EXTRA_SEAL_LENGTH]
}

/// Extract the 65-byte signature from the end of extra_data.
///
/// Returns an error if extra_data is shorter than EXTRA_SEAL_LENGTH.
pub fn extract_signature(extra: &[u8]) -> Result<[u8; 65], ExtraDataError> {
    if extra.len() < EXTRA_SEAL_LENGTH {
        return Err(ExtraDataError::TooShort(EXTRA_SEAL_LENGTH, extra.len()));
    }
    let start = extra.len() - EXTRA_SEAL_LENGTH;
    Ok(extra[start..]
        .try_into()
        .expect("slice has exactly 65 bytes"))
}

/// Parse validator addresses and BLS public keys from an epoch block's extra data.
///
/// At epoch blocks, the extra field contains (after the 32-byte vanity):
///   [validatorNumber: 1 byte][N * (20-byte addr + 48-byte BLS key)]
///
/// The `is_bohr` parameter controls whether a turn-length byte is expected after
/// the validator list (required to correctly locate the end of validator data).
pub fn parse_validators(
    extra: &[u8],
    is_bohr: bool,
) -> Result<Vec<(Address, [u8; 48])>, ExtraDataError> {
    let min_len = EXTRA_VANITY_LENGTH + VALIDATOR_NUMBER_SIZE + EXTRA_SEAL_LENGTH;
    if extra.len() < min_len {
        return Err(ExtraDataError::TooShort(min_len, extra.len()));
    }

    let num_validators = extra[EXTRA_VANITY_LENGTH] as usize;
    let validators_start = EXTRA_VANITY_LENGTH + VALIDATOR_NUMBER_SIZE;
    let validators_end = validators_start + num_validators * VALIDATOR_BYTES_LENGTH;

    // Compute minimum required total length including turn length byte (if post-Bohr)
    // and the trailing seal.
    let mut min_total = validators_end + EXTRA_SEAL_LENGTH;
    if is_bohr {
        min_total += TURN_LENGTH_SIZE;
    }

    if extra.len() < min_total {
        return Err(ExtraDataError::InvalidValidatorCount(num_validators));
    }

    let mut validators = Vec::with_capacity(num_validators);
    for i in 0..num_validators {
        let offset = validators_start + i * VALIDATOR_BYTES_LENGTH;
        let addr_bytes: [u8; 20] = extra[offset..offset + 20]
            .try_into()
            .expect("slice has exactly 20 bytes");
        let bls_bytes: [u8; 48] = extra[offset + 20..offset + 68]
            .try_into()
            .expect("slice has exactly 48 bytes");
        validators.push((Address::from(addr_bytes), bls_bytes));
    }

    Ok(validators)
}

/// Parse the turn-length byte from an epoch block's extra data (post-Bohr).
///
/// The turn-length byte follows immediately after the validator list:
///   vanity[32] + validatorNumber[1] + N*validatorBytes + turnLength[1] + ...
pub fn parse_turn_length(extra: &[u8]) -> Result<u8, ExtraDataError> {
    let min_len = EXTRA_VANITY_LENGTH + VALIDATOR_NUMBER_SIZE + EXTRA_SEAL_LENGTH;
    if extra.len() < min_len {
        return Err(ExtraDataError::TooShort(min_len, extra.len()));
    }

    let num_validators = extra[EXTRA_VANITY_LENGTH] as usize;
    let validators_start = EXTRA_VANITY_LENGTH + VALIDATOR_NUMBER_SIZE;
    let validators_end = validators_start + num_validators * VALIDATOR_BYTES_LENGTH;
    let turn_length_offset = validators_end;

    // Need: turn_length byte + seal at end
    if extra.len() < turn_length_offset + TURN_LENGTH_SIZE + EXTRA_SEAL_LENGTH {
        return Err(ExtraDataError::MissingTurnLength);
    }

    Ok(extra[turn_length_offset])
}

/// Fast-finality vote attestation embedded in the block header's extra field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoteAttestation {
    /// Bitset indicating which validators (by index) contributed to the aggregate signature.
    pub vote_address_set: u64,
    /// Aggregated BLS signature over the vote data.
    pub agg_signature: [u8; 96],
    /// The vote data (source/target block range) that was attested.
    pub data: VoteData,
    /// Reserved for future use.
    pub extra: Vec<u8>,
}

/// Source and target block identifiers for a fast-finality vote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoteData {
    pub source_number: u64,
    pub source_hash: H256,
    pub target_number: u64,
    pub target_hash: H256,
}

impl RLPDecode for VoteData {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (source_number, decoder) = decoder.decode_field("source_number")?;
        let (source_hash, decoder) = decoder.decode_field("source_hash")?;
        let (target_number, decoder) = decoder.decode_field("target_number")?;
        let (target_hash, decoder) = decoder.decode_field("target_hash")?;
        let rest = decoder.finish()?;
        Ok((
            VoteData {
                source_number,
                source_hash,
                target_number,
                target_hash,
            },
            rest,
        ))
    }
}

impl RLPDecode for VoteAttestation {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (vote_address_set, decoder) = decoder.decode_field("vote_address_set")?;
        let (agg_signature, decoder) = decoder.decode_field("agg_signature")?;
        let (data, decoder) = decoder.decode_field("data")?;
        let (extra, decoder) = decoder.decode_field("extra")?;
        let rest = decoder.finish()?;
        Ok((
            VoteAttestation {
                vote_address_set,
                agg_signature,
                data,
                extra,
            },
            rest,
        ))
    }
}

/// Parse the RLP-encoded vote attestation from an extra field, if present.
///
/// For non-epoch blocks, the attestation bytes are:
///   extra[EXTRA_VANITY_LENGTH .. extra.len() - EXTRA_SEAL_LENGTH]
///
/// For epoch blocks (`is_epoch = true`, `is_bohr` controls turn-length presence),
/// the attestation bytes start after: vanity + validatorNumber + N*validators + (turnLength?)
///
/// Returns `Ok(None)` when the extra field is too short to contain attestation data
/// or when the attestation section is empty.
pub fn parse_vote_attestation(
    extra: &[u8],
    is_epoch: bool,
    is_bohr: bool,
) -> Result<Option<VoteAttestation>, ExtraDataError> {
    // Not enough room for anything beyond vanity + seal
    if extra.len() <= EXTRA_VANITY_LENGTH + EXTRA_SEAL_LENGTH {
        return Ok(None);
    }

    let attestation_bytes = if !is_epoch {
        // Non-epoch: attestation occupies vanity..len-seal
        &extra[EXTRA_VANITY_LENGTH..extra.len() - EXTRA_SEAL_LENGTH]
    } else {
        // Epoch: attestation starts after the validator list (and optional turn length)
        let min_header = EXTRA_VANITY_LENGTH + VALIDATOR_NUMBER_SIZE + EXTRA_SEAL_LENGTH;
        if extra.len() < min_header {
            return Ok(None);
        }
        let num_validators = extra[EXTRA_VANITY_LENGTH] as usize;
        let mut attest_start =
            EXTRA_VANITY_LENGTH + VALIDATOR_NUMBER_SIZE + num_validators * VALIDATOR_BYTES_LENGTH;
        if is_bohr {
            attest_start += TURN_LENGTH_SIZE;
        }
        let attest_end = extra.len() - EXTRA_SEAL_LENGTH;
        if attest_end <= attest_start {
            return Ok(None);
        }
        &extra[attest_start..attest_end]
    };

    if attestation_bytes.is_empty() {
        return Ok(None);
    }

    let attestation = VoteAttestation::decode(attestation_bytes)?;
    Ok(Some(attestation))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_extra(vanity: &[u8; 32], body: &[u8], seal: &[u8; 65]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(vanity);
        v.extend_from_slice(body);
        v.extend_from_slice(seal);
        v
    }

    #[test]
    fn strip_and_extract_roundtrip() {
        let mut extra = vec![0u8; EXTRA_VANITY_LENGTH + EXTRA_SEAL_LENGTH];
        extra[EXTRA_VANITY_LENGTH..].copy_from_slice(&[0xab; EXTRA_SEAL_LENGTH]);

        let stripped = strip_signature(&extra);
        assert_eq!(stripped.len(), EXTRA_VANITY_LENGTH);

        let sig = extract_signature(&extra).unwrap();
        assert_eq!(sig, [0xab; 65]);
    }

    #[test]
    fn extract_signature_too_short() {
        let extra = vec![0u8; 10];
        assert!(extract_signature(&extra).is_err());
    }

    #[test]
    fn parse_validators_single_entry() {
        let vanity = [0u8; 32];
        let seal = [0u8; 65];
        // 1 validator: count=1, 20 addr bytes + 48 BLS bytes
        let mut body = Vec::new();
        body.push(1u8); // validator count
        body.extend_from_slice(&[0xAAu8; 20]); // address
        body.extend_from_slice(&[0xBBu8; 48]); // BLS key
        body.push(2u8); // turn length (bohr)

        let extra = make_extra(&vanity, &body, &seal);
        let validators = parse_validators(&extra, true).unwrap();
        assert_eq!(validators.len(), 1);
        assert_eq!(validators[0].0, Address::from([0xAA; 20]));
        assert_eq!(validators[0].1, [0xBBu8; 48]);
    }

    #[test]
    fn parse_turn_length_ok() {
        let vanity = [0u8; 32];
        let seal = [0u8; 65];
        let mut body = Vec::new();
        body.push(1u8); // 1 validator
        body.extend_from_slice(&[0u8; 68]); // 20 + 48 bytes
        body.push(3u8); // turn length = 3

        let extra = make_extra(&vanity, &body, &seal);
        let turn_length = parse_turn_length(&extra).unwrap();
        assert_eq!(turn_length, 3);
    }

    #[test]
    fn strip_signature_too_short() {
        let extra = vec![0u8; 10];
        assert_eq!(strip_signature(&extra), &[] as &[u8]);
    }
}
