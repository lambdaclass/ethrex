//! 0x64 — tmHeaderValidate
//!
//! Validates a Tendermint light-client header (consensus state + new header)
//! for the BSC cross-chain bridge.  Used by the BEP-126 light client for
//! Tendermint v0.31.12 and compatible versions.
//!
//! # Input layout
//!
//! ```text
//! | payload_length (32, last 8 bytes = u64 BE) | payload |
//! ```
//!
//! Where `payload` is:
//! ```text
//! | cs_length (32, last 8 bytes = u64 BE) | consensus_state | tendermint_header |
//! ```
//!
//! ## Consensus state binary format
//!
//! ```text
//! | chainID (32) | height (8) | appHash (32) | curValidatorSetHash (32) |
//! | [{ed25519_pubkey (32), voting_power (8)}...] |
//! ```
//!
//! ## Tendermint header format
//!
//! The header is encoded using the Amino binary codec (Tendermint Go's amino
//! library).  This is a custom length-prefixed encoding that wraps a
//! SignedHeader + two ValidatorSets.  Porting the Amino decoder in full would
//! require re-implementing the Amino format for all Tendermint types, which is
//! outside the scope of this implementation.
//!
//! # Output on success
//!
//! ```text
//! | validatorSetChanged (1) | padding (23) | consensusStateBytesLength (8) | new_consensus_state |
//! ```
//!
//! # Gas
//!
//! 3 000  (`params.TendermintHeaderValidateGas`)
//!
//! # Implementation status
//!
//! Input parsing and gas charging are fully implemented.  The actual
//! Tendermint v0.31.x consensus verification (Amino-decoded header, Ed25519
//! commit-signature checks, >2/3 voting-power threshold) is **not yet
//! implemented** because it requires porting the full Amino codec plus the
//! Tendermint `ValidatorSet.VerifyCommit` / `VerifyFutureCommit` logic.
//!
//! The BNB Beacon Chain was sunset in 2024; this precompile sees little real
//! traffic.  A wrong implementation (accepting invalid headers) would corrupt
//! state permanently — returning `ExecutionReverted` for unverifiable inputs
//! is therefore the correct safe default until the full verifier is ported.
//!
//! Reference: `core/vm/lightclient/v1/types.go` in the BSC repository.

use super::PrecompileError;

/// Gas cost for tmHeaderValidate.  Matches `params.TendermintHeaderValidateGas`.
pub const TM_HEADER_VALIDATE_GAS: u64 = 3_000;

// ── Layout constants (matching contracts_lightclient.go) ─────────────────────

/// Length of the outer 32-byte metadata word that carries the payload length.
const OUTER_META_LENGTH: usize = 32;
/// Byte offset of the u64 payload-length within the 32-byte outer word.
const PAYLOAD_LEN_OFFSET: usize = 24; // last 8 bytes

/// Length of the inner 32-byte metadata word that carries the consensus-state
/// length inside the payload.
const CS_LEN_WORD: usize = 32;
/// Byte offset of the u64 cs-length within that word.
const CS_LEN_OFFSET: usize = 24; // last 8 bytes

// ── Consensus-state field sizes ───────────────────────────────────────────────

const CHAIN_ID_LEN: usize = 32;
const HEIGHT_LEN: usize = 8;
const APP_HASH_LEN: usize = 32;
const CUR_VAL_SET_HASH_LEN: usize = 32;
/// Fixed part of a consensus state (without validators).
const CS_FIXED_LEN: usize = CHAIN_ID_LEN + HEIGHT_LEN + APP_HASH_LEN + CUR_VAL_SET_HASH_LEN;
/// Per-validator size: ed25519 pubkey (32) + voting power (8).
const VALIDATOR_ENTRY_LEN: usize = 32 + 8;

/// Maximum number of validators supported (99, matching the Go reference).
const MAX_VALIDATORS: usize = 99;
/// Maximum consensus state byte length.
const MAX_CS_LEN: usize = CS_FIXED_LEN + MAX_VALIDATORS * VALIDATOR_ENTRY_LEN;

// ── Public interface ──────────────────────────────────────────────────────────

/// Run the tmHeaderValidate precompile.
///
/// Gas is always charged before any error is returned (except
/// [`PrecompileError::NotEnoughGas`]).  Input parsing is fully validated;
/// the Amino-encoded Tendermint header and the actual Ed25519 commit-signature
/// verification are not yet implemented and will return
/// [`PrecompileError::NotImplemented`].
pub fn run(input: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    if gas_limit < TM_HEADER_VALIDATE_GAS {
        return Err(PrecompileError::NotEnoughGas);
    }

    // Parse the outer envelope: 32-byte metadata word carrying payload_length
    // in the last 8 bytes.
    if input.len() <= OUTER_META_LENGTH {
        return Err(PrecompileError::InvalidInput);
    }
    let payload_length = u64::from_be_bytes(
        input[PAYLOAD_LEN_OFFSET..OUTER_META_LENGTH]
            .try_into()
            .expect("slice is exactly 8 bytes"),
    ) as usize;

    if input.len() != OUTER_META_LENGTH + payload_length {
        return Err(PrecompileError::InvalidInput);
    }

    let payload = &input[OUTER_META_LENGTH..];

    // Parse the inner consensus-state length word.
    if payload.len() <= CS_LEN_WORD {
        return Err(PrecompileError::InvalidInput);
    }
    let cs_length = u64::from_be_bytes(
        payload[CS_LEN_OFFSET..CS_LEN_WORD]
            .try_into()
            .expect("slice is exactly 8 bytes"),
    ) as usize;

    // Guard against overflow and ensure there are bytes after the CS for the
    // header.
    let cs_end = CS_LEN_WORD
        .checked_add(cs_length)
        .ok_or(PrecompileError::InvalidInput)?;
    if payload.len() <= cs_end {
        return Err(PrecompileError::InvalidInput);
    }

    let cs_bytes = &payload[CS_LEN_WORD..cs_end];
    let header_bytes = &payload[cs_end..];

    // Validate the consensus-state binary structure.
    parse_consensus_state_v1(cs_bytes)?;

    // The header is amino-encoded.  Without a full Amino decoder for
    // Tendermint v0.31 types, verification cannot proceed.
    if header_bytes.is_empty() {
        return Err(PrecompileError::InvalidInput);
    }

    // TODO: Port Amino decoding + Ed25519 commit-signature verification from
    // `core/vm/lightclient/v1/types.go` (`DecodeHeader`, `ConsensusState.ApplyHeader`).
    // Until then, charge gas and return NotImplemented so callers know the
    // precompile is recognised but unverifiable.
    Err(PrecompileError::NotImplemented)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Parsed representation of a v1 consensus state.
#[allow(dead_code)]
pub(crate) struct ConsensusStateV1<'a> {
    pub chain_id: &'a [u8], // 32 bytes, null-padded
    pub height: u64,
    pub app_hash: &'a [u8],               // 32 bytes
    pub cur_validator_set_hash: &'a [u8], // 32 bytes
    /// Each entry is (ed25519_pubkey[32], voting_power_be[8]).
    pub validators: Vec<(&'a [u8], i64)>,
}

/// Validate and parse the v1 consensus-state binary blob.
///
/// Layout:
/// ```text
/// | chainID (32) | height (8) | appHash (32) | curValidatorSetHash (32) |
/// | [{pubkey (32), votingPower (8)}…] |
/// ```
pub(crate) fn parse_consensus_state_v1(
    input: &[u8],
) -> Result<ConsensusStateV1<'_>, PrecompileError> {
    let len = input.len();

    // Must be at least the fixed portion.
    if len <= CS_FIXED_LEN {
        return Err(PrecompileError::InvalidInput);
    }

    // Variable portion must be an exact multiple of the per-validator size.
    let variable_len = len - CS_FIXED_LEN;
    if !variable_len.is_multiple_of(VALIDATOR_ENTRY_LEN) {
        return Err(PrecompileError::InvalidInput);
    }

    let num_validators = variable_len / VALIDATOR_ENTRY_LEN;
    if num_validators > MAX_VALIDATORS {
        return Err(PrecompileError::InvalidInput);
    }

    // Enforce the absolute size cap (matches `maxConsensusStateLength` in Go).
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

    let app_hash = &input[pos..pos + APP_HASH_LEN];
    pos += APP_HASH_LEN;

    let cur_validator_set_hash = &input[pos..pos + CUR_VAL_SET_HASH_LEN];
    pos += CUR_VAL_SET_HASH_LEN;

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
        validators.push((pubkey, voting_power));
    }

    Ok(ConsensusStateV1 {
        chain_id,
        height,
        app_hash,
        cur_validator_set_hash,
        validators,
    })
}

/// Encode an updated v1 consensus state back to the binary wire format.
///
/// Layout mirrors `DecodeConsensusState` / `EncodeConsensusState` in
/// `core/vm/lightclient/v1/types.go`.
#[allow(dead_code)]
pub(crate) fn encode_consensus_state_v1(
    cs: &ConsensusStateV1<'_>,
) -> Result<Vec<u8>, PrecompileError> {
    let num_validators = cs.validators.len();
    if num_validators > MAX_VALIDATORS {
        return Err(PrecompileError::InvalidInput);
    }

    let total = CS_FIXED_LEN + num_validators * VALIDATOR_ENTRY_LEN;
    let mut out = vec![0u8; total];
    let mut pos = 0;

    // chainID — null-padded to 32 bytes
    let chain_id_len = cs.chain_id.len().min(CHAIN_ID_LEN);
    out[pos..pos + chain_id_len].copy_from_slice(&cs.chain_id[..chain_id_len]);
    pos += CHAIN_ID_LEN;

    out[pos..pos + HEIGHT_LEN].copy_from_slice(&cs.height.to_be_bytes());
    pos += HEIGHT_LEN;

    out[pos..pos + APP_HASH_LEN].copy_from_slice(cs.app_hash);
    pos += APP_HASH_LEN;

    out[pos..pos + CUR_VAL_SET_HASH_LEN].copy_from_slice(cs.cur_validator_set_hash);
    pos += CUR_VAL_SET_HASH_LEN;

    for (pubkey, voting_power) in &cs.validators {
        out[pos..pos + 32].copy_from_slice(pubkey);
        pos += 32;
        out[pos..pos + 8].copy_from_slice(&voting_power.to_be_bytes());
        pos += 8;
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid input envelope from raw cs_bytes + header_bytes.
    fn build_input(cs_bytes: &[u8], header_bytes: &[u8]) -> Vec<u8> {
        // payload = cs_len_word (32) | cs_bytes | header_bytes
        let payload_len = CS_LEN_WORD + cs_bytes.len() + header_bytes.len();

        let mut outer = vec![0u8; OUTER_META_LENGTH];
        outer[PAYLOAD_LEN_OFFSET..OUTER_META_LENGTH]
            .copy_from_slice(&(payload_len as u64).to_be_bytes());

        let mut payload = vec![0u8; CS_LEN_WORD];
        payload[CS_LEN_OFFSET..CS_LEN_WORD].copy_from_slice(&(cs_bytes.len() as u64).to_be_bytes());
        payload.extend_from_slice(cs_bytes);
        payload.extend_from_slice(header_bytes);

        outer.extend_from_slice(&payload);
        outer
    }

    /// Build a minimal valid v1 consensus-state binary blob with `n` validators.
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
            run(&input, TM_HEADER_VALIDATE_GAS - 1),
            Err(PrecompileError::NotEnoughGas)
        );
    }

    #[test]
    fn test_empty_input_rejected() {
        assert_eq!(
            run(&[], TM_HEADER_VALIDATE_GAS),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_short_input_rejected() {
        // Only the outer metadata word (no payload)
        let input = vec![0u8; OUTER_META_LENGTH];
        assert_eq!(
            run(&input, TM_HEADER_VALIDATE_GAS),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_wrong_payload_length_rejected() {
        let mut input = build_input(&build_cs_bytes(1), &[0x00]);
        // Corrupt the payload-length field to a wrong value.
        input[PAYLOAD_LEN_OFFSET..OUTER_META_LENGTH].copy_from_slice(&9999u64.to_be_bytes());
        assert_eq!(
            run(&input, TM_HEADER_VALIDATE_GAS),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_cs_structure_validated() {
        // cs_bytes has wrong length (not CS_FIXED + N*40)
        let bad_cs = vec![0u8; CS_FIXED_LEN + 1]; // 1 leftover byte
        let input = build_input(&bad_cs, &[0x00]);
        assert_eq!(
            run(&input, TM_HEADER_VALIDATE_GAS),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_valid_parse_returns_not_implemented() {
        // A structurally-valid input should pass validation and hit the TODO.
        let input = build_input(&build_cs_bytes(1), &[0x00]);
        assert_eq!(
            run(&input, TM_HEADER_VALIDATE_GAS),
            Err(PrecompileError::NotImplemented)
        );
    }

    #[test]
    fn test_parse_consensus_state_v1_roundtrip() {
        let n = 3;
        let cs_bytes = build_cs_bytes(n);
        let cs = parse_consensus_state_v1(&cs_bytes).unwrap();
        assert_eq!(cs.height, 1);
        assert_eq!(cs.validators.len(), n);

        let encoded = encode_consensus_state_v1(&cs).unwrap();
        assert_eq!(encoded, cs_bytes);
    }
}
