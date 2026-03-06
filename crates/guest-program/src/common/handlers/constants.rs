//! Centralized constants for system contract addresses, event topics,
//! and gas costs used by the common transaction handlers.

use std::sync::LazyLock;

use ethrex_common::{Address, H160, H256};
use ethrex_crypto::keccak::keccak_hash;

// ── System contract addresses ─────────────────────────────────────

/// CommonBridgeL2: 0x000000000000000000000000000000000000ffff
pub const COMMON_BRIDGE_L2_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xff,
]);

/// L2-to-L1 Messenger: 0x000000000000000000000000000000000000fffe
pub const L2_TO_L1_MESSENGER_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfe,
]);

/// Fee Token Registry: 0x000000000000000000000000000000000000fffc
pub const FEE_TOKEN_REGISTRY_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfc,
]);

/// Fee Token Ratio: 0x000000000000000000000000000000000000fffb
pub const FEE_TOKEN_RATIO_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfb,
]);

/// Burn address: 0x0000000000000000000000000000000000000000
pub const BURN_ADDRESS: Address = H160([0u8; 20]);

/// ETH token address used in withdrawal data hash:
/// 0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE
#[allow(clippy::mixed_case_hex_literals)]
pub const ETH_TOKEN_ADDRESS: Address = H160([
    0xEE, 0xee, 0xEE, 0xee, 0xEe, 0xEe, 0xeE, 0xEE, 0xEe, 0xEe, 0xee, 0xEE, 0xEE, 0xee, 0xee, 0xEe,
    0xee, 0xee, 0xee, 0xEE,
]);

// ── Messenger storage layout ──────────────────────────────────────

/// Storage slot 0 of L2ToL1Messenger holds `lastMessageId`.
pub const MESSENGER_LAST_MESSAGE_ID_SLOT: H256 = H256([0u8; 32]);

// ── Event topics (LazyLock) ───────────────────────────────────────

/// keccak256("WithdrawalInitiated(address,address,uint256)")
pub static WITHDRAWAL_INITIATED_TOPIC: LazyLock<H256> =
    LazyLock::new(|| H256::from(keccak_hash(b"WithdrawalInitiated(address,address,uint256)")));

/// keccak256("L1Message(address,bytes32,uint256)")
pub static L1MESSAGE_TOPIC: LazyLock<H256> =
    LazyLock::new(|| H256::from(keccak_hash(b"L1Message(address,bytes32,uint256)")));

// Note: Gas constants (WITHDRAWAL_GAS, ETH_TRANSFER_GAS, SYSTEM_CALL_GAS)
// were removed. The guest program now uses block header gas_used instead of
// fixed constants, because actual EVM gas varies with storage state (cold/warm
// access patterns). See: platform/docs/fixture-data-collection.md (2026-03-06).
