//! Drift guard for the SSZ byte-offset constants in `NativeRollup.sol`.
//!
//! `advance()` reads `provenBlockHash`, `provenStateRoot`, `provenBlockNumber`,
//! `provenGasLimit`, and `slot_number` from the SSZ `StatelessInput` calldata at
//! *fixed byte offsets* declared as constants in `NativeRollup.sol`. If the Rust
//! `ExecutionPayload` / `SszStatelessValidationResult` schema changes (a field is
//! added, reordered, or resized) without updating those constants, the contract
//! would silently read the wrong bytes and commit attacker-controlled values on
//! L1.
//!
//! Instead of re-declaring the offsets here (another hand-maintained copy), this
//! test **parses the constants straight out of `NativeRollup.sol`** and checks
//! each one against the offset **derived from the real SSZ encoding** of a sample
//! payload. So the `.sol` file is the single source of the numbers, and this one
//! test catches drift between the contract, the constants, and the schema.

#![allow(clippy::unwrap_used)]

use ethrex_common::types::stateless_ssz::{
    Bytes20, ExecutionPayload, ExecutionRequests, NewPayloadRequest, SszExecutionWitness,
    SszStatelessInput, SszStatelessValidationResult,
};
use libssz::SszEncode;

/// Path to the contract, relative to this test crate's manifest dir (`<repo>/test`).
const NATIVE_ROLLUP_SOL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../crates/l2/contracts/src/nativeRollup/l1/NativeRollup.sol"
);

fn read_contract() -> String {
    std::fs::read_to_string(NATIVE_ROLLUP_SOL)
        .unwrap_or_else(|e| panic!("failed to read {NATIVE_ROLLUP_SOL}: {e}"))
}

/// Parse `uint<N> constant <name> = <value>;` out of the contract source.
///
/// Anchors on `constant <name>` so it matches the declaration rather than a comment,
/// then asserts the declared type is some `uint`. Accepts any width and both decimal
/// and `0x` hex literals: pinning `uint16 constant EXPECTED_SCHEMA_ID = 0x1501` needs
/// both, and an earlier `uint256`-and-decimal-only version silently could not read it,
/// which left the schema id as the one constant with no drift guard.
fn sol_uint_const(src: &str, name: &str) -> usize {
    let needle = format!("constant {name}");
    let start = src
        .find(&needle)
        .unwrap_or_else(|| panic!("`constant {name}` not found in NativeRollup.sol"));

    let declared_type = src[..start]
        .trim_end()
        .rsplit(char::is_whitespace)
        .next()
        .unwrap_or("");
    assert!(
        declared_type.starts_with("uint"),
        "{name} is declared `{declared_type}`, expected a uint type"
    );

    let after = &src[start + needle.len()..];
    let eq = after
        .find('=')
        .unwrap_or_else(|| panic!("no `=` after constant {name}"));
    let literal = after[eq + 1..].trim_start();

    // Solidity allows `_` digit separators (e.g. `300_000`) and `0x` hex literals.
    let (radix, digits) = match literal
        .strip_prefix("0x")
        .or_else(|| literal.strip_prefix("0X"))
    {
        Some(hex) => (16, hex),
        None => (10, literal),
    };
    let value: String = digits
        .chars()
        .take_while(|c| c.is_digit(radix) || *c == '_')
        .filter(|c| *c != '_')
        .collect();
    assert!(!value.is_empty(), "could not parse value for {name}");
    usize::from_str_radix(&value, radix)
        .unwrap_or_else(|_| panic!("could not parse value for {name}"))
}

fn u32_le(bytes: &[u8], off: usize) -> usize {
    (bytes[off] as usize)
        | ((bytes[off + 1] as usize) << 8)
        | ((bytes[off + 2] as usize) << 16)
        | ((bytes[off + 3] as usize) << 24)
}

/// Sample payload with a distinctive sentinel in every field the contract reads.
fn sample_execution_payload() -> ExecutionPayload {
    ExecutionPayload {
        parent_hash: [0x11; 32],
        fee_recipient: Bytes20([0x22; 20]),
        state_root: [0x33; 32],
        receipts_root: [0x44; 32],
        logs_bloom: vec![0u8; 256].try_into().expect("logs_bloom"),
        prev_randao: [0x55; 32],
        block_number: 7,
        gas_limit: 30_000_000,
        gas_used: 21_000,
        timestamp: 1_700_000_000,
        extra_data: vec![].try_into().expect("extra_data"),
        base_fee_per_gas: [0u8; 32],
        block_hash: [0x66; 32],
        transactions: Vec::new().into(),
        withdrawals: Vec::new().into(),
        blob_gas_used: 0,
        excess_blob_gas: 0,
        block_access_list: Vec::new().into(),
        slot_number: 0x7843,
    }
}

/// Encode a sample `statelessInputBytes`: the 2-byte big-endian schema id then
/// the SSZ body, exactly as `build_ssz_stateless_input` emits it. The prefix is
/// modelled here on purpose — the contract rebases every absolute read past it,
/// so a guard over an unprefixed body would not catch a framing mismatch.
fn encode_sample_input() -> Vec<u8> {
    let input = SszStatelessInput {
        new_payload_request: NewPayloadRequest {
            execution_payload: sample_execution_payload(),
            versioned_hashes: Vec::new().into(),
            parent_beacon_block_root: [0u8; 32],
            execution_requests: ExecutionRequests {
                deposits: Vec::new().into(),
                withdrawals: Vec::new().into(),
                consolidations: Vec::new().into(),
                builder_deposits: Vec::new().into(),
                builder_exits: Vec::new().into(),
            },
        },
        witness: SszExecutionWitness {
            state: vec![].try_into().expect("state"),
            codes: vec![].try_into().expect("codes"),
            headers: vec![].try_into().expect("headers"),
        },
        chain_id: 1,
        public_keys: vec![].try_into().expect("public_keys"),
    };
    let mut buf = ethrex_common::types::stateless_ssz::STATELESS_INPUT_SCHEMA_ID
        .to_be_bytes()
        .to_vec();
    input.ssz_append(&mut buf);
    buf
}

/// The `ExecutionPayload` offset constants in `NativeRollup.sol` must point to
/// the right field in the real SSZ encoding.
#[test]
fn sol_ep_offsets_match_encoding() {
    let src = read_contract();
    let buf = encode_sample_input();
    let prefix_len = sol_uint_const(&src, "INPUT_SCHEMA_PREFIX_LEN");
    assert_eq!(prefix_len, 2, "INPUT_SCHEMA_PREFIX_LEN must be 2");
    assert_eq!(
        u16::from_be_bytes([buf[0], buf[1]]),
        ethrex_common::types::stateless_ssz::STATELESS_INPUT_SCHEMA_ID,
        "sample input must carry the schema-id prefix the contract requires"
    );

    // StatelessInput fixed part: 4 offsets; new_payload_request is field 0.
    // SSZ offsets are body-relative, so rebase past the prefix as `advance()` does.
    let npr_abs = prefix_len + u32_le(&buf, prefix_len);
    // NewPayloadRequest fixed prefix: execution_payload offset @ npr_abs.
    let ep_abs = npr_abs + u32_le(&buf, npr_abs);

    let parent_hash_off = sol_uint_const(&src, "EP_PARENT_HASH_OFFSET");
    assert_eq!(
        &buf[ep_abs + parent_hash_off..ep_abs + parent_hash_off + 32],
        &[0x11; 32],
        "EP_PARENT_HASH_OFFSET does not point at parent_hash"
    );

    let state_root_off = sol_uint_const(&src, "EP_STATE_ROOT_OFFSET");
    assert_eq!(
        &buf[ep_abs + state_root_off..ep_abs + state_root_off + 32],
        &[0x33; 32],
        "EP_STATE_ROOT_OFFSET does not point at state_root"
    );

    let block_number_off = sol_uint_const(&src, "EP_BLOCK_NUMBER_OFFSET");
    assert_eq!(
        u64::from_le_bytes(
            buf[ep_abs + block_number_off..ep_abs + block_number_off + 8]
                .try_into()
                .unwrap()
        ),
        7,
        "EP_BLOCK_NUMBER_OFFSET does not point at block_number"
    );

    let gas_limit_off = sol_uint_const(&src, "EP_GAS_LIMIT_OFFSET");
    assert_eq!(
        u64::from_le_bytes(
            buf[ep_abs + gas_limit_off..ep_abs + gas_limit_off + 8]
                .try_into()
                .unwrap()
        ),
        30_000_000,
        "EP_GAS_LIMIT_OFFSET does not point at gas_limit"
    );

    let block_hash_off = sol_uint_const(&src, "EP_BLOCK_HASH_OFFSET");
    assert_eq!(
        &buf[ep_abs + block_hash_off..ep_abs + block_hash_off + 32],
        &[0x66; 32],
        "EP_BLOCK_HASH_OFFSET does not point at block_hash"
    );

    let slot_number_off = sol_uint_const(&src, "EP_SLOT_NUMBER_OFFSET");
    assert_eq!(
        u64::from_le_bytes(
            buf[ep_abs + slot_number_off..ep_abs + slot_number_off + 8]
                .try_into()
                .unwrap()
        ),
        0x7843,
        "EP_SLOT_NUMBER_OFFSET does not point at slot_number"
    );

    // EP_FIXED_PREFIX_LEN: with every variable-length field empty, the
    // block_access_list offset slot (the 4 bytes immediately before slot_number)
    // holds the fixed-prefix length. This pins EP_FIXED_PREFIX_LEN to the encoding.
    let fixed_prefix_len = sol_uint_const(&src, "EP_FIXED_PREFIX_LEN");
    assert_eq!(
        u32_le(&buf, ep_abs + slot_number_off - 4),
        fixed_prefix_len,
        "EP_FIXED_PREFIX_LEN does not equal the encoded fixed-prefix length"
    );
}

/// The `StatelessValidationResult` offset constants in `NativeRollup.sol` must
/// match the real SSZ encoding of the result the precompile returns.
#[test]
fn sol_result_offsets_match_encoding() {
    let src = read_contract();
    let result = SszStatelessValidationResult {
        new_payload_request_root: [0xAA; 32],
        successful_validation: true,
        chain_id: 0x1122334455667788,
        schema_id: ethrex_common::types::stateless_ssz::STATELESS_INPUT_SCHEMA_ID,
    };
    let mut buf = Vec::new();
    result.ssz_append(&mut buf);

    let success_off = sol_uint_const(&src, "RESULT_SUCCESS_OFFSET");
    assert_eq!(
        buf[success_off], 1,
        "RESULT_SUCCESS_OFFSET does not point at successful_validation"
    );

    // Since execution-specs #3278 the result is entirely fixed-size, so
    // RESULT_FIXED_LEN is the exact encoded length and both remaining fields are
    // direct reads — there is no chain_config offset to dereference.
    let fixed_len = sol_uint_const(&src, "RESULT_FIXED_LEN");
    assert_eq!(
        buf.len(),
        fixed_len,
        "RESULT_FIXED_LEN must equal the exact encoded result length"
    );

    let chain_id_off = sol_uint_const(&src, "RESULT_CHAIN_ID_OFFSET");
    let chain_id = u64::from_le_bytes(buf[chain_id_off..chain_id_off + 8].try_into().unwrap());
    assert_eq!(
        chain_id, 0x1122334455667788,
        "RESULT_CHAIN_ID_OFFSET does not point at chain_id"
    );

    let schema_off = sol_uint_const(&src, "RESULT_SCHEMA_ID_OFFSET");
    let schema_id = u16::from_le_bytes(buf[schema_off..schema_off + 2].try_into().unwrap());
    assert_eq!(
        schema_id,
        ethrex_common::types::stateless_ssz::STATELESS_INPUT_SCHEMA_ID,
        "RESULT_SCHEMA_ID_OFFSET does not point at schema_id"
    );
}

/// Drift guard for the relayer-gas overhead constant, which is duplicated in
/// Solidity (`NativeRollup.sol`) and Rust (`block_producer.rs`). `sendL1Message`'s
/// includability cap and the producer's relayer-tx sizing both use it and MUST
/// agree — a silent drift wedges the bridge (Rust > Sol) or makes a valid message
/// un-includable (Sol > Rust). This pins the `.sol` value to the Rust source of
/// truth, the same way `sol_ep_offsets_match_encoding` pins the SSZ offsets.
#[test]
fn sol_relayer_gas_allowance_matches_rust() {
    let src = read_contract();
    let sol = sol_uint_const(&src, "RELAYER_GAS_BODY_ALLOWANCE") as u64;
    assert_eq!(
        sol,
        ethrex_l2::sequencer::native_rollup::block_producer::RELAYER_GAS_BODY_ALLOWANCE,
        "RELAYER_GAS_BODY_ALLOWANCE drifted between NativeRollup.sol and block_producer.rs"
    );
}

/// Drift guard for the `l2GasLimit` upper bound. `advance()` re-executes the L2
/// block inside a single L1 transaction, so the contract's `MAX_L2_GAS_LIMIT` must
/// equal the L1 per-transaction gas cap (EIP-7825, `TX_MAX_GAS_LIMIT_AMSTERDAM`).
/// If the Rust constant changes, this pins the `.sol` copy to it so the two can't
/// silently drift and let a too-large (unadvanceable) `l2GasLimit` be deployed.
#[test]
fn sol_max_l2_gas_limit_matches_rust() {
    let src = read_contract();
    let sol = sol_uint_const(&src, "MAX_L2_GAS_LIMIT") as u64;
    assert_eq!(
        sol,
        ethrex_common::constants::TX_MAX_GAS_LIMIT_AMSTERDAM,
        "MAX_L2_GAS_LIMIT in NativeRollup.sol drifted from TX_MAX_GAS_LIMIT_AMSTERDAM"
    );
}

/// The deployed contract rejects any proof whose schema id is not its own compiled-in
/// `EXPECTED_SCHEMA_ID`. Bumping `STATELESS_INPUT_SCHEMA_ID` for a new fork without
/// redeploying therefore reverts every `advance()` and wedges the rollup, and because
/// the contract is immutable that is not recoverable in place.
///
/// The other assertions in this file build their buffers from the Rust constant and so
/// cannot catch this: comparing the Rust value to itself is a tautology with respect to
/// the Solidity literal. This reads the `.sol` value directly.
#[test]
fn sol_expected_schema_id_matches_rust() {
    let src = read_contract();
    let sol = sol_uint_const(&src, "EXPECTED_SCHEMA_ID") as u16;
    assert_eq!(
        sol,
        ethrex_common::types::stateless_ssz::STATELESS_INPUT_SCHEMA_ID,
        "EXPECTED_SCHEMA_ID in NativeRollup.sol drifted from STATELESS_INPUT_SCHEMA_ID; \
         deploying this would revert every advance()"
    );
}
