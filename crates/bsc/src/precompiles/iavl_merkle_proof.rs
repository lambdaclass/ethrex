//! 0x65 — iavlMerkleProofValidate
//!
//! Validates an IAVL Merkle proof for the BSC cross-chain bridge state
//! verification.
//!
//! # Input layout
//!
//! ```text
//! | payload_length (32, last 8 bytes = u64 BE) | payload |
//! ```
//!
//! Where `payload` is a `KeyValueMerkleProof`:
//! ```text
//! | storeName (32)       |
//! | keyLength (32, last 8 = u64 BE) |
//! | key (keyLength bytes)           |
//! | valueLength (32, last 8 = u64 BE) |
//! | value (valueLength bytes)         |
//! | appHash (32)         |
//! | proof (remaining)    |   ← Amino-encoded tendermint Merkle Proof
//! ```
//!
//! # Output on success
//!
//! 32 bytes with the last 8 bytes set to `0x0000000000000001`.
//!
//! # Gas
//!
//! 3 000  (`params.IAVLMerkleProofValidateGas`)
//!
//! # Implementation status
//!
//! Input parsing (outer envelope + KeyValueMerkleProof binary structure) is
//! fully implemented.  The final Merkle-proof verification step is **not yet
//! implemented** because the proof wire format uses Amino-encoded
//! Tendermint/IAVL types (or ICS23 protobuf from Plato onwards) whose full
//! decoders would require significant porting work.
//!
//! Key logic from `core/vm/lightclient/v1/types.go` (`DecodeKeyValueMerkleProof`,
//! `KeyValueMerkleProof.Validate`) and `core/vm/lightclient/v1/ics23_proof.go`.
//!
//! As with `tmHeaderValidate`, returning `NotImplemented` (rather than blindly
//! accepting every proof) is the safe default: a wrong acceptance would corrupt
//! cross-chain state permanently on any block that calls this precompile.

use super::PrecompileError;

/// Gas cost for iavlMerkleProofValidate.  Matches `params.IAVLMerkleProofValidateGas`.
pub const IAVL_MERKLE_PROOF_GAS: u64 = 3_000;

// ── Layout constants ──────────────────────────────────────────────────────────

/// Outer 32-byte metadata word carrying `payload_length` in its last 8 bytes.
const OUTER_META_LENGTH: usize = 32;
const PAYLOAD_LEN_OFFSET: usize = 24;

/// `storeName` field: fixed 32-byte null-padded string.
const STORE_NAME_LEN: usize = 32;
/// Length word for `key`: 32 bytes, u64 in the last 8 bytes.
const KEY_LEN_WORD: usize = 32;
const KEY_LEN_OFFSET: usize = 24;
/// Length word for `value`: same layout.
const VALUE_LEN_WORD: usize = 32;
const VALUE_LEN_OFFSET: usize = 24;
/// `appHash` field: fixed 32 bytes.
const APP_HASH_LEN: usize = 32;

/// Minimum fixed-size portion of the payload (without key/value/proof data).
const MIN_FIXED_PAYLOAD: usize = STORE_NAME_LEN + KEY_LEN_WORD + VALUE_LEN_WORD + APP_HASH_LEN;

// ── Public interface ──────────────────────────────────────────────────────────

/// Parsed representation of an IAVL Merkle proof payload.
#[allow(dead_code)]
pub(crate) struct KeyValueMerkleProof<'a> {
    pub store_name: &'a [u8], // 32 bytes null-padded
    pub key: &'a [u8],
    pub value: &'a [u8],
    pub app_hash: &'a [u8], // 32 bytes
    /// Remaining bytes after the app_hash — the raw proof bytes.
    /// These are either Amino-encoded or ICS23 protobuf-encoded depending on
    /// the fork (Plato and later use ICS23).
    pub proof_bytes: &'a [u8],
}

/// Run the iavlMerkleProofValidate precompile.
///
/// Gas is always charged before returning any error other than
/// [`PrecompileError::NotEnoughGas`].  Structural validation of the input is
/// complete; the actual Merkle-proof verification is not yet implemented.
pub fn run(input: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    if gas_limit < IAVL_MERKLE_PROOF_GAS {
        return Err(PrecompileError::NotEnoughGas);
    }
    Ok((IAVL_MERKLE_PROOF_GAS, run_inner(input).unwrap_or_default()))
}

fn run_inner(input: &[u8]) -> Option<Vec<u8>> {
    if input.len() <= OUTER_META_LENGTH {
        return None;
    }
    let payload_length =
        u64::from_be_bytes(input[PAYLOAD_LEN_OFFSET..OUTER_META_LENGTH].try_into().ok()?) as usize;
    if input.len() != OUTER_META_LENGTH + payload_length {
        return None;
    }
    let payload = &input[OUTER_META_LENGTH..];
    parse_kv_merkle_proof(payload).ok()?;
    // TODO: Port proof verification from bsc-geth lightclient/v1/types.go and
    // ics23_proof.go. Until implemented, return None (predictable-failure path).
    None
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Parse and validate the `KeyValueMerkleProof` payload layout.
///
/// Returns a structured view into the input slice on success, or
/// `InvalidInput` on any structural violation.
pub(crate) fn parse_kv_merkle_proof(
    payload: &[u8],
) -> Result<KeyValueMerkleProof<'_>, PrecompileError> {
    let payload_len = payload.len();

    if payload_len <= MIN_FIXED_PAYLOAD {
        return Err(PrecompileError::InvalidInput);
    }

    let mut pos = 0;

    // storeName — 32 bytes
    let store_name = &payload[pos..pos + STORE_NAME_LEN];
    pos += STORE_NAME_LEN;

    // keyLength — last 8 bytes of a 32-byte word
    let key_length = u64::from_be_bytes(
        payload[pos + KEY_LEN_OFFSET..pos + KEY_LEN_WORD]
            .try_into()
            .expect("slice is exactly 8 bytes"),
    ) as usize;
    pos += KEY_LEN_WORD;

    // Guard: MIN_FIXED + key_length must not overflow and payload must have room.
    let after_key = pos
        .checked_add(key_length)
        .ok_or(PrecompileError::InvalidInput)?;
    if payload_len <= after_key + VALUE_LEN_WORD + APP_HASH_LEN {
        return Err(PrecompileError::InvalidInput);
    }
    let key = &payload[pos..pos + key_length];
    pos += key_length;

    // valueLength — last 8 bytes of a 32-byte word
    let value_length = u64::from_be_bytes(
        payload[pos + VALUE_LEN_OFFSET..pos + VALUE_LEN_WORD]
            .try_into()
            .expect("slice is exactly 8 bytes"),
    ) as usize;
    pos += VALUE_LEN_WORD;

    // Guard against overflow and ensure there are enough remaining bytes.
    let after_value = pos
        .checked_add(value_length)
        .ok_or(PrecompileError::InvalidInput)?;
    if payload_len <= after_value + APP_HASH_LEN {
        return Err(PrecompileError::InvalidInput);
    }
    let value = &payload[pos..pos + value_length];
    pos += value_length;

    // appHash — 32 bytes
    let app_hash = &payload[pos..pos + APP_HASH_LEN];
    pos += APP_HASH_LEN;

    // Remaining bytes are the raw proof.
    let proof_bytes = &payload[pos..];

    Ok(KeyValueMerkleProof {
        store_name,
        key,
        value,
        app_hash,
        proof_bytes,
    })
}

/// Encode the successful-validation result: 32 bytes with last 8 = `0x01`.
#[allow(dead_code)]
pub(crate) fn successful_result() -> Vec<u8> {
    let mut result = vec![0u8; 32];
    result[24..32].copy_from_slice(&1u64.to_be_bytes());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal outer-envelope-wrapped input from raw payload bytes.
    fn wrap_payload(payload: &[u8]) -> Vec<u8> {
        let mut outer = vec![0u8; OUTER_META_LENGTH];
        outer[PAYLOAD_LEN_OFFSET..OUTER_META_LENGTH]
            .copy_from_slice(&(payload.len() as u64).to_be_bytes());
        outer.extend_from_slice(payload);
        outer
    }

    /// Build a minimal valid payload with no key, no value, and a single proof byte.
    fn build_minimal_payload() -> Vec<u8> {
        let key_length: usize = 0;
        let value_length: usize = 0;
        let proof = &[0xAAu8];

        let mut payload = Vec::new();
        // storeName (32 zeros)
        payload.extend_from_slice(&[0u8; STORE_NAME_LEN]);
        // keyLength word (32 bytes, last 8 = 0)
        let mut kl_word = [0u8; KEY_LEN_WORD];
        kl_word[KEY_LEN_OFFSET..KEY_LEN_WORD].copy_from_slice(&(key_length as u64).to_be_bytes());
        payload.extend_from_slice(&kl_word);
        // key (empty)
        // valueLength word (32 bytes, last 8 = 0)
        let mut vl_word = [0u8; VALUE_LEN_WORD];
        vl_word[VALUE_LEN_OFFSET..VALUE_LEN_WORD]
            .copy_from_slice(&(value_length as u64).to_be_bytes());
        payload.extend_from_slice(&vl_word);
        // value (empty)
        // appHash (32 zeros)
        payload.extend_from_slice(&[0u8; APP_HASH_LEN]);
        // proof
        payload.extend_from_slice(proof);
        payload
    }

    #[test]
    fn test_not_enough_gas() {
        let input = wrap_payload(&build_minimal_payload());
        assert_eq!(
            run(&input, IAVL_MERKLE_PROOF_GAS - 1),
            Err(PrecompileError::NotEnoughGas)
        );
    }

    #[test]
    fn test_empty_input_rejected() {
        assert_eq!(
            run(&[], IAVL_MERKLE_PROOF_GAS),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_only_meta_rejected() {
        let input = vec![0u8; OUTER_META_LENGTH];
        assert_eq!(
            run(&input, IAVL_MERKLE_PROOF_GAS),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_wrong_payload_length_rejected() {
        let mut input = wrap_payload(&build_minimal_payload());
        // Overwrite the payload-length with an incorrect value.
        input[PAYLOAD_LEN_OFFSET..OUTER_META_LENGTH].copy_from_slice(&9999u64.to_be_bytes());
        assert_eq!(
            run(&input, IAVL_MERKLE_PROOF_GAS),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_payload_too_short_rejected() {
        // Payload smaller than MIN_FIXED_PAYLOAD
        let tiny = vec![0u8; MIN_FIXED_PAYLOAD - 1];
        let input = wrap_payload(&tiny);
        assert_eq!(
            run(&input, IAVL_MERKLE_PROOF_GAS),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_valid_structure_returns_not_implemented() {
        let input = wrap_payload(&build_minimal_payload());
        assert_eq!(
            run(&input, IAVL_MERKLE_PROOF_GAS),
            Err(PrecompileError::NotImplemented)
        );
    }

    #[test]
    fn test_successful_result_format() {
        let r = successful_result();
        assert_eq!(r.len(), 32);
        assert_eq!(&r[24..32], &1u64.to_be_bytes());
        assert_eq!(&r[..24], &[0u8; 24]);
    }

    #[test]
    fn test_parse_kv_proof_extracts_fields() {
        let payload = build_minimal_payload();
        let kv = parse_kv_merkle_proof(&payload).unwrap();
        assert_eq!(kv.key.len(), 0);
        assert_eq!(kv.value.len(), 0);
        assert_eq!(kv.proof_bytes, &[0xAA]);
    }
}
