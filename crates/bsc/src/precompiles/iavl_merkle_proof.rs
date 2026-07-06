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
//! | proof (remaining)    |   ← ICS23 tendermint Merkle Proof (protobuf)
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
//! Implements the **Plato** (pure-ICS23) variant active on BSC mainnet since
//! block 30,720,096 (and unchanged by every later fork). Membership proofs are
//! fully verified via the `ics23` crate (which matches `bnb-chain/ics23`
//! bit-for-bit, including the IAVL-prefix hardening from the Oct-2022 bridge
//! exploit fix). Two-op structure, key/appHash checks, and gas match bsc-geth
//! `iavlMerkleProofValidatePlato`.
//!
//! Key logic from `core/vm/lightclient/v1/types.go` (`DecodeKeyValueMerkleProof`,
//! `KeyValueMerkleProof.Validate`), `core/vm/lightclient/v1/ics23_proof.go`, and
//! `core/vm/lightclient/v1/multistoreproof.go` (`Ics23ProofRuntime`).
//!
//! Remaining gap: **absence** proofs (empty value → `VerifyAbsence`) are not yet
//! ported and are rejected. The legacy pre-Plato amino runtimes are not
//! implemented (mainnet is long past Plato).

use super::PrecompileError;
use ics23::{
    calculate_existence_root, iavl_spec, tendermint_spec, verify_membership, CommitmentProof,
    HostFunctionsManager,
};
use prost::Message as _;

/// Gas cost for iavlMerkleProofValidate.  Matches `params.IAVLMerkleProofValidateGas`.
pub const IAVL_MERKLE_PROOF_GAS: u64 = 3_000;

/// ICS23 op type for the IAVL substore membership proof (op 0).
const OP_ICS23_IAVL: &str = "ics23:iavl";
/// ICS23 op type for the simple-merkle multistore membership proof (op 1).
const OP_ICS23_SIMPLE: &str = "ics23:simple";

/// Tendermint `crypto/merkle.Proof` — a list of `ProofOp`s. The outer wrapper
/// around the two ICS23 `CommitmentProof`s. Decoded with prost so wire handling
/// matches bsc-geth's gogoproto `Unmarshal`.
#[derive(Clone, PartialEq, ::prost::Message)]
struct MerkleProof {
    #[prost(message, repeated, tag = "1")]
    ops: Vec<MerkleProofOp>,
}

/// Tendermint `crypto/merkle.ProofOp`. `data` is a marshalled `ics23.CommitmentProof`.
#[derive(Clone, PartialEq, ::prost::Message)]
struct MerkleProofOp {
    #[prost(string, tag = "1")]
    op_type: String,
    #[prost(bytes = "vec", tag = "2")]
    key: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    data: Vec<u8>,
}

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
/// The flat 3000 gas is charged on success. Any parse/verification failure
/// returns an `Err`, which the CALL layer turns into an all-gas-burn failure —
/// matching bsc-geth, where a precompile error consumes all forwarded gas.
pub fn run(input: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError> {
    if gas_limit < IAVL_MERKLE_PROOF_GAS {
        return Err(PrecompileError::NotEnoughGas);
    }
    // bsc-geth's `iavlMerkleProofValidate.Run` returns an error on parse/validation
    // failures, and the BSC CALL implementation burns ALL forwarded gas when
    // a precompile errors. See the matching note in `tm_header_validate::run`.
    let output = run_inner(input)?;
    Ok((IAVL_MERKLE_PROOF_GAS, output))
}

fn run_inner(input: &[u8]) -> Result<Vec<u8>, PrecompileError> {
    if input.len() <= OUTER_META_LENGTH {
        return Err(PrecompileError::InvalidInput);
    }
    let payload_length = u64::from_be_bytes(
        input[PAYLOAD_LEN_OFFSET..OUTER_META_LENGTH]
            .try_into()
            .map_err(|_| PrecompileError::InvalidInput)?,
    ) as usize;
    if input.len() != OUTER_META_LENGTH + payload_length {
        return Err(PrecompileError::InvalidInput);
    }
    let payload = &input[OUTER_META_LENGTH..];
    let kvmp = parse_kv_merkle_proof(payload)?;
    verify_kv_merkle_proof(&kvmp)?;
    Ok(successful_result())
}

/// Verify a decoded `KeyValueMerkleProof` under the Plato (pure-ICS23) rules.
///
/// Mirrors bsc-geth `KeyValueMerkleProof.Validate` + `ProofRuntime.Verify` with
/// `Ics23ProofRuntime`: the outer `merkle.Proof` must carry exactly two
/// `CommitmentOp`s, in order:
///   - op0 `ics23:iavl`  — proves `key → value` in the substore's IAVL tree;
///     the proof's computed root is that substore's root.
///   - op1 `ics23:simple` — proves `storeName → substoreRoot` in the top-level
///     simple-merkle multistore; the computed root must equal `appHash`.
///
/// Each op's key must match the corresponding keypath segment (`/storeName/key`).
/// Returns `Ok(())` only when both memberships verify and the final root equals
/// `appHash`; any structural or verification failure is an `Err`, which the CALL
/// layer turns into an all-gas-burn failure (matching bsc-geth).
fn verify_kv_merkle_proof(kvmp: &KeyValueMerkleProof<'_>) -> Result<(), PrecompileError> {
    // storeName is a 32-byte field, NUL-trimmed (bsc-geth `bytes.Trim(_, "\x00")`).
    let store_name = trim_nuls(kvmp.store_name);

    let proof = MerkleProof::decode(kvmp.proof_bytes).map_err(|_| PrecompileError::InvalidInput)?;
    if proof.ops.len() != 2 {
        return Err(PrecompileError::InvalidInput);
    }
    let op_iavl = &proof.ops[0];
    let op_simple = &proof.ops[1];
    if op_iavl.op_type != OP_ICS23_IAVL || op_simple.op_type != OP_ICS23_SIMPLE {
        return Err(PrecompileError::InvalidInput);
    }
    // Keypath consumption: op0 consumes the store key, op1 the store name.
    if op_iavl.key.as_slice() != kvmp.key || op_simple.key.as_slice() != store_name {
        return Err(PrecompileError::InvalidInput);
    }

    // Absence proofs (empty value) go through bsc-geth `VerifyAbsence` (a
    // non-existence CommitmentProof); not yet ported. Reject rather than risk a
    // wrong acceptance — same conservative stance as before for this sub-case.
    if kvmp.value.is_empty() {
        return Err(PrecompileError::NotImplemented);
    }

    let cp_iavl = CommitmentProof::decode(op_iavl.data.as_slice())
        .map_err(|_| PrecompileError::InvalidInput)?;
    let cp_simple = CommitmentProof::decode(op_simple.data.as_slice())
        .map_err(|_| PrecompileError::InvalidInput)?;

    // op0 (ics23:iavl): the substore root is the proof's own computed root.
    let Some(ics23::commitment_proof::Proof::Exist(exist)) = &cp_iavl.proof else {
        return Err(PrecompileError::InvalidInput);
    };
    let substore_root = calculate_existence_root::<HostFunctionsManager>(exist)
        .map_err(|_| PrecompileError::InvalidInput)?;
    if !verify_membership::<HostFunctionsManager>(
        &cp_iavl,
        &iavl_spec(),
        &substore_root,
        kvmp.key,
        kvmp.value,
    ) {
        return Err(PrecompileError::InvalidInput);
    }

    // op1 (ics23:simple): storeName → substoreRoot, and the computed root must
    // equal appHash. Passing appHash as the expected root enforces that equality.
    let app_hash = kvmp.app_hash.to_vec();
    if !verify_membership::<HostFunctionsManager>(
        &cp_simple,
        &tendermint_spec(),
        &app_hash,
        store_name,
        &substore_root,
    ) {
        return Err(PrecompileError::InvalidInput);
    }

    Ok(())
}

/// Trim leading and trailing NUL bytes, matching Go `bytes.Trim(b, "\x00")`.
fn trim_nuls(b: &[u8]) -> &[u8] {
    let start = b.iter().position(|&x| x != 0).unwrap_or(b.len());
    let end = b.iter().rposition(|&x| x != 0).map_or(start, |i| i + 1);
    &b[start..end]
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
    fn test_garbage_proof_rejected() {
        // Structurally valid envelope but the proof bytes aren't a valid
        // merkle.Proof protobuf -> rejected (was NotImplemented while stubbed).
        let input = wrap_payload(&build_minimal_payload());
        assert_eq!(
            run(&input, IAVL_MERKLE_PROOF_GAS),
            Err(PrecompileError::InvalidInput)
        );
    }

    /// Assemble a full precompile input (outer length prefix + payload) from
    /// the KeyValueMerkleProof components.
    fn build_input(
        store: &[u8],
        key: &[u8],
        value: &[u8],
        app_hash: &[u8],
        proof: &[u8],
    ) -> Vec<u8> {
        fn word_be(n: u64) -> [u8; 32] {
            let mut w = [0u8; 32];
            w[24..].copy_from_slice(&n.to_be_bytes());
            w
        }
        let mut store32 = [0u8; 32];
        store32[..store.len()].copy_from_slice(store);

        let mut payload = Vec::new();
        payload.extend_from_slice(&store32);
        payload.extend_from_slice(&word_be(key.len() as u64));
        payload.extend_from_slice(key);
        payload.extend_from_slice(&word_be(value.len() as u64));
        payload.extend_from_slice(value);
        payload.extend_from_slice(app_hash);
        payload.extend_from_slice(proof);

        let mut input = Vec::new();
        input.extend_from_slice(&word_be(payload.len() as u64));
        input.extend_from_slice(&payload);
        input
    }

    #[test]
    fn plato_ics23_membership_golden_vector() {
        // bsc-geth core/vm/contracts_lightclient_test.go, TestIcs23ProofPlato:
        // store "ibc", key "wind", value "blows" — a valid two-op ICS23 proof
        // that verifies against the given appHash and returns the 0x..01 sentinel.
        let key = hex::decode("77696e64").unwrap();
        let value = hex::decode("626c6f7773").unwrap();
        let app_hash =
            hex::decode("ae6d1123fc362b3297bfb19c9f9fabbcbd1e2555b923dead261905b8a2ff6db6")
                .unwrap();
        let proof = hex::decode(
            "0a300a0a69637332333a6961766c120477696e641a1c0a1a0a0477696e641205626c6f77731a0b08011801\
             20012a030002040a9d010a0c69637332333a73696d706c6512036962631a87010a84010a036962631220141\
             acb8632cfb808f293f2649cb9aabaca74fc18640900ffd0d48e2994b2a1521a090801180120012a010022270\
             8011201011a205f0ba08283de309300409486e978a3ea59d82bccc838b07c7d39bd87c16a503422270801120\
             1011a20455b81ef5591150bd24d3e57a769f65518b16de93487f0fab02271b3d69e2852",
        )
        .unwrap();

        let input = build_input(b"ibc", &key, &value, &app_hash, &proof);
        let (gas, out) = run(&input, 100_000).expect("valid ICS23 membership proof must verify");
        assert_eq!(gas, IAVL_MERKLE_PROOF_GAS);
        assert_eq!(out, successful_result());
    }

    #[test]
    fn plato_ics23_wrong_apphash_rejected() {
        // Same proof but a tampered appHash must fail verification.
        let key = hex::decode("77696e64").unwrap();
        let value = hex::decode("626c6f7773").unwrap();
        let mut app_hash =
            hex::decode("ae6d1123fc362b3297bfb19c9f9fabbcbd1e2555b923dead261905b8a2ff6db6")
                .unwrap();
        app_hash[0] ^= 0xff; // corrupt
        let proof = hex::decode(
            "0a300a0a69637332333a6961766c120477696e641a1c0a1a0a0477696e641205626c6f77731a0b08011801\
             20012a030002040a9d010a0c69637332333a73696d706c6512036962631a87010a84010a036962631220141\
             acb8632cfb808f293f2649cb9aabaca74fc18640900ffd0d48e2994b2a1521a090801180120012a010022270\
             8011201011a205f0ba08283de309300409486e978a3ea59d82bccc838b07c7d39bd87c16a503422270801120\
             1011a20455b81ef5591150bd24d3e57a769f65518b16de93487f0fab02271b3d69e2852",
        )
        .unwrap();

        let input = build_input(b"ibc", &key, &value, &app_hash, &proof);
        assert_eq!(run(&input, 100_000), Err(PrecompileError::InvalidInput));
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
