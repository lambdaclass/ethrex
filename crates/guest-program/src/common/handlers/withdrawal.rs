//! Withdrawal (L2->L1) handler.
//!
//! Fixes the three root causes of SP1 proof verification failure for withdrawals:
//!   1. BURN_ADDRESS must receive the withdrawn value (EVM: bridge -> BURN_ADDRESS.call{value})
//!   2. L1Messenger.lastMessageId must be incremented (storage slot 0)
//!   3. Correct withdrawal logs must be generated (WithdrawalInitiated + L1Message)
//!
//! The `get_block_l1_messages()` function in `crates/l2/common/src/messages.rs`
//! extracts L1Message events (topic = L1MESSAGE_TOPIC, address = MESSENGER_ADDRESS)
//! from receipts to compute `l1_out_messages_merkle_root`. The Log 2 generated here
//! must exactly match that filter.

use bytes::Bytes;

use ethrex_common::types::{Log, Transaction};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::keccak::keccak_hash;

use super::constants::{
    BURN_ADDRESS, COMMON_BRIDGE_L2_ADDRESS, ETH_TOKEN_ADDRESS, L1MESSAGE_TOPIC,
    L2_TO_L1_MESSENGER_ADDRESS, MESSENGER_LAST_MESSAGE_ID_SLOT, WITHDRAWAL_GAS,
    WITHDRAWAL_INITIATED_TOPIC,
};
use crate::common::app_execution::AppCircuitError;
use crate::common::app_state::AppState;

/// Handle a withdrawal (L2->L1) transaction via CommonBridgeL2.
///
/// Performs the three state changes that the EVM executes:
///   1. sender -= value (gas is handled separately by `gas_fee` module)
///   2. BURN_ADDRESS += value (EVM: bridge -> BURN_ADDRESS.call{value}(""))
///   3. Messenger.lastMessageId += 1 (storage slot 0)
///
/// Returns `(gas_used, new_message_id)` for receipt and log generation.
pub fn handle_withdrawal(
    state: &mut AppState,
    tx: &Transaction,
    sender: Address,
) -> Result<(u64, U256), AppCircuitError> {
    let value = tx.value();

    // 1. sender -= value
    if !value.is_zero() {
        state.debit_balance(sender, value)?;
    }

    // 2. BURN_ADDRESS += value
    if !value.is_zero() {
        state.credit_balance(BURN_ADDRESS, value)?;
    }

    // 3. Messenger.lastMessageId += 1
    let current_id =
        state.get_storage(L2_TO_L1_MESSENGER_ADDRESS, MESSENGER_LAST_MESSAGE_ID_SLOT)?;
    let new_id = current_id + U256::one();
    state.set_storage(
        L2_TO_L1_MESSENGER_ADDRESS,
        MESSENGER_LAST_MESSAGE_ID_SLOT,
        new_id,
    )?;

    Ok((WITHDRAWAL_GAS, new_id))
}

/// Generate the two EVM-matching event logs for a withdrawal transaction.
///
/// Log 1: `WithdrawalInitiated(address indexed sender, address indexed receiver, uint256 indexed amount)`
///         emitted by CommonBridgeL2 (0xffff)
///
/// Log 2: `L1Message(address indexed senderOnL2, bytes32 indexed data, uint256 indexed messageId)`
///         emitted by L2ToL1Messenger (0xfffe)
///
/// The `data_hash` in Log 2 is `keccak256(abi.encodePacked(ETH_TOKEN, ETH_TOKEN, receiverOnL1, value))`.
pub fn generate_withdrawal_logs(sender: Address, tx: &Transaction, message_id: U256) -> Vec<Log> {
    let value = tx.value();
    let data = tx.data();

    // Parse receiver from calldata: data[16..36] = ABI-encoded address
    let receiver_on_l1 = if data.len() >= 36 {
        Address::from_slice(&data[16..36])
    } else {
        sender // fallback: send back to sender
    };

    // Log 1: WithdrawalInitiated from bridge (0xffff) -- all params indexed
    let log1 = Log {
        address: COMMON_BRIDGE_L2_ADDRESS,
        topics: vec![
            *WITHDRAWAL_INITIATED_TOPIC,
            addr_to_h256(sender),
            addr_to_h256(receiver_on_l1),
            u256_to_h256(value),
        ],
        data: Bytes::new(),
    };

    // Log 2: L1Message from Messenger (0xfffe) -- all params indexed
    // data_hash = keccak256(abi.encodePacked(ETH_TOKEN, ETH_TOKEN, receiverOnL1, value))
    let data_hash = compute_withdrawal_data_hash(receiver_on_l1, value);
    let log2 = Log {
        address: L2_TO_L1_MESSENGER_ADDRESS,
        topics: vec![
            *L1MESSAGE_TOPIC,
            addr_to_h256(COMMON_BRIDGE_L2_ADDRESS),
            data_hash,
            u256_to_h256(message_id),
        ],
        data: Bytes::new(),
    };

    vec![log1, log2]
}

/// Convert an address to H256 (left-padded with zeros).
fn addr_to_h256(addr: Address) -> H256 {
    let mut bytes = [0u8; 32];
    bytes[12..32].copy_from_slice(addr.as_bytes());
    H256(bytes)
}

/// Convert U256 to H256 (big-endian).
fn u256_to_h256(value: U256) -> H256 {
    H256(value.to_big_endian())
}

/// Compute the withdrawal data hash used in the L1Message event.
///
/// `keccak256(abi.encodePacked(ETH_TOKEN_ADDRESS, ETH_TOKEN_ADDRESS, receiverOnL1, value))`
///
/// This matches the Solidity: `keccak256(abi.encodePacked(tokenOnL2, tokenOnL1, receiver, amount))`
/// where for ETH withdrawals both tokens are ETH_TOKEN_ADDRESS.
fn compute_withdrawal_data_hash(receiver: Address, value: U256) -> H256 {
    // abi.encodePacked produces tightly packed bytes:
    //   20 bytes (ETH_TOKEN) + 20 bytes (ETH_TOKEN) + 20 bytes (receiver) + 32 bytes (value)
    let mut packed = Vec::with_capacity(92);
    packed.extend_from_slice(ETH_TOKEN_ADDRESS.as_bytes()); // 20 bytes
    packed.extend_from_slice(ETH_TOKEN_ADDRESS.as_bytes()); // 20 bytes
    packed.extend_from_slice(receiver.as_bytes()); // 20 bytes
    packed.extend_from_slice(&value.to_big_endian()); // 32 bytes

    H256::from(keccak_hash(&packed))
}
