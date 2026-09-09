//! The Hegotá testnet's active EIP set, tested rather than asserted.
//!
//! The claim "this chain runs five EIPs" is a claim about *activation*, not
//! about code absence: the tree still carries the payer-TXPARAM and
//! derived-slot extensions, each behind its own chain-config field. A field left
//! unset is the whole of the guarantee, so each one is exercised here against a
//! `ChainConfig` shaped like the published genesis — Hegotá on, everything else
//! absent.
//!
//! A surface that silently activated on a default would be consensus-visible
//! divergence from a client reading the same genesis: a chain split on the first
//! block that uses it, with nothing in the genesis file to point at.

use ethrex_common::types::{BlockHeader, ChainConfig, DEFAULT_AA_VOPS_SLOT_COUNT, Fork};
use ethrex_levm::environment::EVMConfig;

/// The testnet's chain config: Hegotá from genesis, every optional surface
/// absent. Mirrors what the published genesis must contain.
fn testnet_config() -> ChainConfig {
    ChainConfig {
        chain_id: 8141,
        shanghai_time: Some(0),
        cancun_time: Some(0),
        prague_time: Some(0),
        osaka_time: Some(0),
        amsterdam_time: Some(0),
        hegota_time: Some(0),
        // The three fields that must stay absent.
        payer_txparam_time: None,
        derived_slot_time: None,
        aa_vops_slot_count: None,
        ..Default::default()
    }
}

fn header_at(timestamp: u64, slot_number: Option<u64>) -> BlockHeader {
    BlockHeader {
        timestamp,
        slot_number,
        ..Default::default()
    }
}

#[test]
fn the_config_reaches_hegota_and_stops_there() {
    let config = testnet_config();
    assert_eq!(config.fork(0), Fork::Hegota);
    assert!(config.payer_txparam_time.is_none());
    assert!(config.derived_slot_time.is_none());
    assert!(config.aa_vops_slot_count.is_none());
}

#[test]
fn txparam_0x12_is_inactive_while_payer_txparam_time_is_unset() {
    // `TXPARAM(0x12)` (resolved payer) is an ethrex extension, not in the
    // EIP-8141 draft. With the knob absent the index falls through to the
    // exceptional halt, so a chain reading this genesis sees the draft's
    // TXPARAM set and nothing more.
    let config = testnet_config();
    let evm_config = EVMConfig::new_from_chain_config(&config, &header_at(0, None));
    assert!(
        !evm_config.payer_txparam_active,
        "TXPARAM(0x12) must stay an invalid index without the knob"
    );
}

#[test]
fn slotnum_comes_from_the_header_and_is_never_derived() {
    // EIP-7843: the beacon slot is the CL's to supply. `derivedSlotTime` is a
    // single-EL devnet fallback that computes it from the block timestamp, and
    // it must be absent here — a derived slot that disagreed with a CL-aware
    // client's would move every recent-root key.
    let config = testnet_config();
    assert!(!config.is_derived_slot_activated(0));

    // A CL-supplied slot is used verbatim.
    let header = header_at(1_000_000, Some(42));
    assert_eq!(
        config.effective_slot_number(header.slot_number, header.timestamp),
        42
    );

    // Without one, the slot stays 0 rather than being derived from the
    // timestamp, however large that timestamp is.
    let no_slot = header_at(1_000_000, None);
    assert_eq!(
        config.effective_slot_number(no_slot.slot_number, no_slot.timestamp),
        0,
        "an unset derivedSlotTime must leave the slot at 0, not derive one"
    );
}

#[test]
fn aa_vops_slot_count_defaults_to_four() {
    // EIP-8369's `AA_VOPS_SLOT_COUNT` is the top of the candidate range and the
    // worst case for replay. The field is absent from the genesis, so the
    // default is what every client must agree on; a joining client that read a
    // different one would classify different transactions as Profile 2
    // candidates and reach a different omission verdict.
    let config = testnet_config();
    assert_eq!(config.aa_vops_slot_count(), 4);
    assert_eq!(config.aa_vops_slot_count(), DEFAULT_AA_VOPS_SLOT_COUNT);
}
