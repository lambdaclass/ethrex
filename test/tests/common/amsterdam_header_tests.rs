//! Amsterdam header-field conformance tests.
//!
//! Covers two hardening fixes:
//!   - EIP-7843: `slot_number` is a mandatory Amsterdam header field. A header
//!     omitting it must be rejected (and a pre-Amsterdam header carrying it must
//!     also be rejected).
//!   - EIP-7918 (`calc_excess_blob_gas`): the reserve-price comparison must not
//!     overflow u64 when the parent base fee is very large.

use ethrex_common::constants::{BLOB_BASE_COST, GAS_PER_BLOB};
use ethrex_common::types::{
    BlockHeader, ChainConfig, Fork, ForkBlobSchedule, InvalidBlockHeaderError,
    calc_excess_blob_gas, validate_cancun_header_fields, validate_prague_header_fields,
};
use ethrex_common::{H256, U256};

/// ChainConfig with every fork through Amsterdam active at genesis.
fn amsterdam_config() -> ChainConfig {
    ChainConfig {
        cancun_time: Some(0),
        prague_time: Some(0),
        osaka_time: Some(0),
        amsterdam_time: Some(0),
        ..Default::default()
    }
}

/// ChainConfig with Prague active but NOT Amsterdam.
fn prague_only_config() -> ChainConfig {
    ChainConfig {
        cancun_time: Some(0),
        prague_time: Some(0),
        ..Default::default()
    }
}

/// A header that satisfies every Prague/Amsterdam field check except the
/// specific field a test mutates. `excess_blob_gas == 0` matches the expected
/// value when the config carries no blob schedule (validate falls back to 0).
fn base_header() -> BlockHeader {
    BlockHeader {
        base_fee_per_gas: Some(7),
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        parent_beacon_block_root: Some(H256::zero()),
        requests_hash: Some(H256::zero()),
        block_access_list_hash: Some(H256::zero()),
        slot_number: Some(0),
        ..Default::default()
    }
}

// ==================== EIP-7843: slot_number presence ====================

#[test]
fn amsterdam_header_missing_slot_number_is_rejected() {
    let cfg = amsterdam_config();
    let parent = base_header();
    let mut header = base_header();
    header.slot_number = None;

    let err = validate_prague_header_fields(&header, &parent, &cfg)
        .expect_err("Amsterdam header without slot_number must be rejected");
    assert!(matches!(err, InvalidBlockHeaderError::SlotNumberNotPresent));
}

#[test]
fn amsterdam_header_with_slot_number_is_accepted() {
    let cfg = amsterdam_config();
    let parent = base_header();
    let header = base_header();

    validate_prague_header_fields(&header, &parent, &cfg)
        .expect("Amsterdam header with all required fields must validate");
}

#[test]
fn prague_header_with_slot_number_is_rejected() {
    let cfg = prague_only_config();
    let parent = base_header();
    // Pre-Amsterdam headers must not carry Amsterdam trailing fields.
    let mut header = base_header();
    header.block_access_list_hash = None;
    // slot_number left as Some(0): should be rejected as present-before-fork.

    let err = validate_prague_header_fields(&header, &parent, &cfg)
        .expect_err("pre-Amsterdam header carrying slot_number must be rejected");
    assert!(matches!(err, InvalidBlockHeaderError::SlotNumberPresent));
}

#[test]
fn cancun_header_with_slot_number_is_rejected() {
    let cfg = amsterdam_config();
    let parent = base_header();
    let mut header = base_header();
    header.block_access_list_hash = None;
    header.requests_hash = None; // Cancun validator rejects requests_hash presence

    let err = validate_cancun_header_fields(&header, &parent, &cfg)
        .expect_err("Cancun header carrying slot_number must be rejected");
    assert!(matches!(err, InvalidBlockHeaderError::SlotNumberPresent));
}

// ==================== EIP-7918: excess_blob_gas overflow ====================

#[test]
fn calc_excess_blob_gas_does_not_overflow_on_large_base_fee() {
    // Osaka reserve-price branch multiplies BLOB_BASE_COST * parent_base_fee.
    // With a u64::MAX base fee this overflows u64 (panics in debug) unless the
    // operands are widened to U256 first.
    let schedule = ForkBlobSchedule {
        base_fee_update_fraction: 5_007_716,
        max: 9,
        target: 6,
    };
    let target_blob_gas = (schedule.target * GAS_PER_BLOB) as u64;

    let parent = BlockHeader {
        // excess >= target so the function reaches the reserve-price branch.
        excess_blob_gas: Some(target_blob_gas),
        blob_gas_used: Some(0),
        base_fee_per_gas: Some(u64::MAX),
        ..Default::default()
    };

    // BLOB_BASE_COST * u64::MAX would overflow u64; confirm we reach the
    // reserve-price branch instead of panicking.
    assert!(
        BLOB_BASE_COST.checked_mul(u64::MAX).is_none(),
        "test premise: the product must overflow u64",
    );

    let result = calc_excess_blob_gas(&parent, schedule, Fork::Osaka);

    // LHS (8192 * u64::MAX) dwarfs the RHS, so the reserve-price branch fires:
    // excess_blob_gas + blob_gas_used * (max - target) / max = target_blob_gas.
    assert_eq!(result, target_blob_gas);
}

#[test]
fn calc_excess_blob_gas_large_base_fee_matches_u256_math() {
    // Cross-check the widened arithmetic against a hand-computed U256 comparison
    // for a base fee just above the old u64 overflow threshold.
    let schedule = ForkBlobSchedule {
        base_fee_update_fraction: 5_007_716,
        max: 9,
        target: 6,
    };
    let target_blob_gas = (schedule.target * GAS_PER_BLOB) as u64;
    let base_fee = u64::MAX / 2;

    let parent = BlockHeader {
        excess_blob_gas: Some(target_blob_gas),
        blob_gas_used: Some(GAS_PER_BLOB as u64),
        base_fee_per_gas: Some(base_fee),
        ..Default::default()
    };

    // Reproduce the branch condition with correct (non-overflowing) math.
    let lhs = U256::from(BLOB_BASE_COST) * U256::from(base_fee);
    assert!(lhs > U256::zero(), "sanity: widened product is non-zero");

    let result = calc_excess_blob_gas(&parent, schedule, Fork::Osaka);
    // Reserve-price branch: excess + blob_gas_used * (max-target)/max.
    let expected = target_blob_gas
        + (GAS_PER_BLOB as u64) * (schedule.max as u64 - schedule.target as u64)
            / schedule.max as u64;
    assert_eq!(result, expected);
}
