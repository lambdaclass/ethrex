//! Tests for EIP-7708: ETH Transfers Emit a Log
//!
//! Behavioral coverage (transfer logs, burn logs, fork gating, log shape) is
//! exercised by the EELS state and blockchain ef-tests at
//! `tests/amsterdam/eip7708_eth_transfer_logs/`. The single check kept here
//! verifies the source-level constants match the spec keccak preimages, which
//! ef-tests cannot validate because fixtures embed the hashes directly.

use ethrex_common::constants::SYSTEM_ADDRESS;
use ethrex_levm::constants::{BURN_EVENT_TOPIC, TRANSFER_EVENT_TOPIC};

#[test]
fn test_topic_hash_and_system_address_constants() {
    let expected_transfer_hash = ethrex_common::utils::keccak(b"Transfer(address,address,uint256)");
    assert_eq!(
        TRANSFER_EVENT_TOPIC, expected_transfer_hash,
        "TRANSFER_EVENT_TOPIC should match keccak256('Transfer(address,address,uint256)')"
    );

    let expected_burn_hash = ethrex_common::utils::keccak(b"Burn(address,uint256)");
    assert_eq!(
        BURN_EVENT_TOPIC, expected_burn_hash,
        "BURN_EVENT_TOPIC should match keccak256('Burn(address,uint256)')"
    );

    let expected_bytes: [u8; 20] = [
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    ];
    assert_eq!(
        SYSTEM_ADDRESS.as_bytes(),
        &expected_bytes,
        "SYSTEM_ADDRESS should be 0xfffffffffffffffffffffffffffffffffffffffe"
    );
}
