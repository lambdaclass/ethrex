//! 0x67 — cometBFTLightBlockValidate
//!
//! Validates a CometBFT light block for the BSC cross-chain bridge.  Used by
//! the BEP-341 light client for CometBFT v0.37.0 and compatible versions.
//!
//! # Input layout
//!
//! ```text
//! | cs_length (32, last 8 bytes = u64 BE) | consensus_state | light_block |
//! ```
//!
//! ## Consensus state binary format (v2)
//!
//! ```text
//! | chainID (32) | height (8) | nextValidatorSetHash (32) |
//! | [{ed25519_pubkey (32), voting_power (8), relayer_address (20), bls_key (48)}…] |
//! ```
//!
//! Each validator entry is 108 bytes.  Maximum 99 validators.
//!
//! ## Light block format
//!
//! The light block is encoded as a protobuf `LightBlock` message from the
//! `cometbft.types.v1` protobuf package.  Decoding it requires the full
//! CometBFT protobuf schema plus the signature-verification logic from
//! `cometbft/light`.
//!
//! # Output on success
//!
//! ```text
//! | validatorSetChanged (1) | padding (23) | consensusStateBytesLength (8) | new_consensus_state |
//! ```
//!
//! # Gas
//!
//! 3 000  (`params.CometBFTLightBlockValidateGas`)
//!
//! # Implementation status
//!
//! Input parsing (outer envelope + v2 consensus-state binary structure) is
//! fully implemented.  The protobuf `LightBlock` decoding and the
//! CometBFT commit-signature verification (Ed25519, >2/3 voting power) are
//! **not yet implemented** — they would require either vendoring the full
//! CometBFT protobuf types via `prost` or linking against a C library.
//!
//! Reference: `core/vm/lightclient/v2/lightclient.go`
//! (`DecodeLightBlockValidationInput`, `ConsensusState.ApplyLightBlock`,
//! `EncodeLightBlockValidationResult`).

use super::PrecompileError;

/// Gas cost for cometBFTLightBlockValidate.  Matches `params.CometBFTLightBlockValidateGas`.
pub const COMETBFT_VALIDATE_GAS: u64 = 3_000;

// ── Layout constants ──────────────────────────────────────────────────────────

/// Outer 32-byte word carrying the consensus-state length in its last 8 bytes.
const CS_LEN_WORD: usize = 32;
const CS_LEN_OFFSET: usize = 24;

/// Fixed-size prefix of the v2 consensus state.
const CHAIN_ID_LEN: usize = 32;
const HEIGHT_LEN: usize = 8;
const NEXT_VAL_SET_HASH_LEN: usize = 32;
const CS_FIXED_LEN: usize = CHAIN_ID_LEN + HEIGHT_LEN + NEXT_VAL_SET_HASH_LEN;

/// Per-validator entry: ed25519 pubkey (32) + voting power (8) + relayer
/// address (20) + BLS key (48) = 108 bytes.
const VALIDATOR_ENTRY_LEN: usize = 32 + 8 + 20 + 48;

/// Maximum number of validators (99, matching the Go reference).
const MAX_VALIDATORS: usize = 99;
/// Maximum v2 consensus-state length.
const MAX_CS_LEN: usize = CS_FIXED_LEN + MAX_VALIDATORS * VALIDATOR_ENTRY_LEN;

// ── Public interface ──────────────────────────────────────────────────────────

/// A single validator entry in a v2 consensus state.
#[allow(dead_code)]
pub(crate) struct ValidatorEntryV2<'a> {
    pub pubkey: &'a [u8], // 32 bytes (ed25519)
    pub voting_power: i64,
    pub relayer_address: &'a [u8], // 20 bytes
    pub bls_key: &'a [u8],         // 48 bytes
}

/// Parsed representation of a v2 (CometBFT) consensus state.
#[allow(dead_code)]
pub(crate) struct ConsensusStateV2<'a> {
    pub chain_id: &'a [u8], // 32 bytes, null-padded
    pub height: u64,
    pub next_validator_set_hash: &'a [u8], // 32 bytes
    pub validators: Vec<ValidatorEntryV2<'a>>,
}

/// Run the cometBFTLightBlockValidate precompile.
///
/// Gas is charged before returning any error other than
/// [`PrecompileError::NotEnoughGas`].  Structural parsing of the consensus
/// state is complete; protobuf `LightBlock` decoding and Ed25519
/// commit-signature verification are not yet implemented.
pub fn run(input: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    if gas_limit < COMETBFT_VALIDATE_GAS {
        return Err(PrecompileError::NotEnoughGas);
    }

    if input.is_empty() {
        return Err(PrecompileError::InvalidInput);
    }

    // The input starts directly with a 32-byte cs_length word (no outer
    // payload-length envelope, unlike 0x64 and 0x65).
    // Reference: `DecodeLightBlockValidationInput` in lightclient/v2/lightclient.go.
    if input.len() <= CS_LEN_WORD {
        return Err(PrecompileError::InvalidInput);
    }

    let cs_length = u64::from_be_bytes(
        input[CS_LEN_OFFSET..CS_LEN_WORD]
            .try_into()
            .expect("slice is exactly 8 bytes"),
    ) as usize;

    // Guard against overflow and ensure light-block bytes follow.
    let cs_end = CS_LEN_WORD
        .checked_add(cs_length)
        .ok_or(PrecompileError::InvalidInput)?;
    if input.len() <= cs_end {
        return Err(PrecompileError::InvalidInput);
    }

    let cs_bytes = &input[CS_LEN_WORD..cs_end];
    let light_block_bytes = &input[cs_end..];

    // Validate the consensus-state structure.
    parse_consensus_state_v2(cs_bytes)?;

    // The light block is protobuf-encoded (cometbft.types.v1.LightBlock).
    // Without the full CometBFT protobuf schema, decoding is not possible.
    if light_block_bytes.is_empty() {
        return Err(PrecompileError::InvalidInput);
    }

    // TODO: Decode the protobuf LightBlock (cometbft.types.v1.LightBlock) and
    // run `ConsensusState.ApplyLightBlock` from lightclient/v2/lightclient.go.
    // This includes:
    //   1. Parsing the SignedHeader, ValidatorSet, and NextValidatorSet from
    //      the protobuf LightBlock message.
    //   2. VerifyCommitLight / VerifyCommitLightTrusting for adjacent/non-adjacent
    //      blocks using Ed25519 signatures.
    //   3. Computing the new consensus state and encoding it via
    //      `EncodeLightBlockValidationResult`.
    Err(PrecompileError::NotImplemented)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Validate and parse the v2 consensus-state binary blob.
///
/// Layout:
/// ```text
/// | chainID (32) | height (8) | nextValidatorSetHash (32) |
/// | [{pubkey (32), votingPower (8), relayerAddress (20), blsKey (48)}…] |
/// ```
pub(crate) fn parse_consensus_state_v2(
    input: &[u8],
) -> Result<ConsensusStateV2<'_>, PrecompileError> {
    let len = input.len();

    if len <= CS_FIXED_LEN {
        return Err(PrecompileError::InvalidInput);
    }

    let variable_len = len - CS_FIXED_LEN;
    if !variable_len.is_multiple_of(VALIDATOR_ENTRY_LEN) {
        return Err(PrecompileError::InvalidInput);
    }

    let num_validators = variable_len / VALIDATOR_ENTRY_LEN;
    if num_validators > MAX_VALIDATORS {
        return Err(PrecompileError::InvalidInput);
    }

    // Enforce the absolute size cap.
    if len > MAX_CS_LEN {
        return Err(PrecompileError::InvalidInput);
    }

    let mut pos = 0;

    let chain_id = &input[pos..pos + CHAIN_ID_LEN];
    pos += CHAIN_ID_LEN;

    let height = u64::from_be_bytes(
        input[pos..pos + HEIGHT_LEN]
            .try_into()
            .expect("slice is exactly 8 bytes"),
    );
    pos += HEIGHT_LEN;

    let next_validator_set_hash = &input[pos..pos + NEXT_VAL_SET_HASH_LEN];
    pos += NEXT_VAL_SET_HASH_LEN;

    let mut validators = Vec::with_capacity(num_validators);
    for _ in 0..num_validators {
        let pubkey = &input[pos..pos + 32];
        pos += 32;
        let voting_power = i64::from_be_bytes(
            input[pos..pos + 8]
                .try_into()
                .expect("slice is exactly 8 bytes"),
        );
        pos += 8;
        let relayer_address = &input[pos..pos + 20];
        pos += 20;
        let bls_key = &input[pos..pos + 48];
        pos += 48;

        validators.push(ValidatorEntryV2 {
            pubkey,
            voting_power,
            relayer_address,
            bls_key,
        });
    }

    Ok(ConsensusStateV2 {
        chain_id,
        height,
        next_validator_set_hash,
        validators,
    })
}

/// Encode a v2 consensus state back to its binary wire format.
#[allow(dead_code)]
pub(crate) fn encode_consensus_state_v2(
    cs: &ConsensusStateV2<'_>,
) -> Result<Vec<u8>, PrecompileError> {
    let num_validators = cs.validators.len();
    if num_validators > MAX_VALIDATORS {
        return Err(PrecompileError::InvalidInput);
    }

    let total = CS_FIXED_LEN + num_validators * VALIDATOR_ENTRY_LEN;
    let mut out = vec![0u8; total];
    let mut pos = 0;

    let chain_id_len = cs.chain_id.len().min(CHAIN_ID_LEN);
    out[pos..pos + chain_id_len].copy_from_slice(&cs.chain_id[..chain_id_len]);
    pos += CHAIN_ID_LEN;

    out[pos..pos + HEIGHT_LEN].copy_from_slice(&cs.height.to_be_bytes());
    pos += HEIGHT_LEN;

    out[pos..pos + NEXT_VAL_SET_HASH_LEN].copy_from_slice(cs.next_validator_set_hash);
    pos += NEXT_VAL_SET_HASH_LEN;

    for v in &cs.validators {
        out[pos..pos + 32].copy_from_slice(v.pubkey);
        pos += 32;
        out[pos..pos + 8].copy_from_slice(&v.voting_power.to_be_bytes());
        pos += 8;
        out[pos..pos + 20].copy_from_slice(v.relayer_address);
        pos += 20;
        out[pos..pos + 48].copy_from_slice(v.bls_key);
        pos += 48;
    }

    Ok(out)
}

/// Encode the light-block validation result.
///
/// ```text
/// | validatorSetChanged (1) | padding (23) | consensusStateBytesLength (8) | new_consensus_state |
/// ```
#[allow(dead_code)]
pub(crate) fn encode_result(validator_set_changed: bool, consensus_state_bytes: &[u8]) -> Vec<u8> {
    let mut header = vec![0u8; 32];
    if validator_set_changed {
        header[0] = 0x01;
    }
    header[24..32].copy_from_slice(&(consensus_state_bytes.len() as u64).to_be_bytes());
    let mut result = header;
    result.extend_from_slice(consensus_state_bytes);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a valid input: cs_length word + cs_bytes + light_block_bytes.
    fn build_input(cs_bytes: &[u8], light_block: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; CS_LEN_WORD];
        out[CS_LEN_OFFSET..CS_LEN_WORD].copy_from_slice(&(cs_bytes.len() as u64).to_be_bytes());
        out.extend_from_slice(cs_bytes);
        out.extend_from_slice(light_block);
        out
    }

    /// Build a minimal valid v2 consensus-state bytes with `n` validators.
    fn build_cs_bytes(n: usize) -> Vec<u8> {
        let total = CS_FIXED_LEN + n * VALIDATOR_ENTRY_LEN;
        let mut v = vec![0u8; total];
        // height = 1
        v[CHAIN_ID_LEN..CHAIN_ID_LEN + HEIGHT_LEN].copy_from_slice(&1u64.to_be_bytes());
        v
    }

    #[test]
    fn test_not_enough_gas() {
        let input = build_input(&build_cs_bytes(1), &[0x00]);
        assert_eq!(
            run(&input, COMETBFT_VALIDATE_GAS - 1),
            Err(PrecompileError::NotEnoughGas)
        );
    }

    #[test]
    fn test_empty_input_rejected() {
        assert_eq!(
            run(&[], COMETBFT_VALIDATE_GAS),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_only_cs_len_word_rejected() {
        // Input is exactly 32 bytes — no room for cs_bytes or light block.
        let input = vec![0u8; CS_LEN_WORD];
        assert_eq!(
            run(&input, COMETBFT_VALIDATE_GAS),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_cs_length_overflow_rejected() {
        // Set cs_length to u64::MAX to trigger overflow guard.
        let mut input = vec![0u8; CS_LEN_WORD + 1];
        input[CS_LEN_OFFSET..CS_LEN_WORD].copy_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(
            run(&input, COMETBFT_VALIDATE_GAS),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_cs_structure_validated() {
        // cs_bytes length not aligned to VALIDATOR_ENTRY_LEN
        let bad_cs = vec![0u8; CS_FIXED_LEN + 1];
        let input = build_input(&bad_cs, &[0x00]);
        assert_eq!(
            run(&input, COMETBFT_VALIDATE_GAS),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_valid_parse_returns_not_implemented() {
        let input = build_input(&build_cs_bytes(1), &[0x00]);
        assert_eq!(
            run(&input, COMETBFT_VALIDATE_GAS),
            Err(PrecompileError::NotImplemented)
        );
    }

    #[test]
    fn test_parse_cs_v2_roundtrip() {
        let n = 2;
        let cs_bytes = build_cs_bytes(n);
        let cs = parse_consensus_state_v2(&cs_bytes).unwrap();
        assert_eq!(cs.height, 1);
        assert_eq!(cs.validators.len(), n);

        let encoded = encode_consensus_state_v2(&cs).unwrap();
        assert_eq!(encoded, cs_bytes);
    }

    #[test]
    fn test_encode_result_layout() {
        let cs_data = vec![0x42u8; 10];
        let result = encode_result(true, &cs_data);
        assert_eq!(result[0], 0x01);
        assert_eq!(&result[1..24], &[0u8; 23]);
        assert_eq!(&result[24..32], &10u64.to_be_bytes());
        assert_eq!(&result[32..], &cs_data[..]);
    }

    #[test]
    fn test_encode_result_layout_false() {
        let cs_data = vec![0x42u8; 5];
        let result = encode_result(false, &cs_data);
        assert_eq!(result[0], 0x00);
    }
}
