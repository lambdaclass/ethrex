//! Event generation for ZkDex contract operations.
//!
//! Produces EVM-compatible [`Log`] entries that match the Solidity contract's
//! event emissions exactly, ensuring receipt root consistency.
//!
//! ## Events
//!
//! - `NoteStateChange(bytes32 indexed note, State state)` — emitted by all
//!   note-mutating operations.
//! - `OrderTaken(uint256 indexed orderId, bytes32 takerNoteToMaker, bytes32 parentNote)`
//! - `OrderSettled(uint256 indexed orderId, bytes32 rewardNote, bytes32 paymentNote, bytes32 changeNote)`

use bytes::Bytes;
use ethrex_common::types::Log;
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::keccak::keccak_hash;

// ── Event topic constants ───────────────────────────────────────

/// `NoteStateChange(bytes32,uint8)` — note state transition event.
pub fn note_state_change_topic() -> H256 {
    H256::from(keccak_hash(b"NoteStateChange(bytes32,uint8)"))
}

/// `OrderTaken(uint256,bytes32,bytes32)` — order taken by taker.
pub fn order_taken_topic() -> H256 {
    H256::from(keccak_hash(b"OrderTaken(uint256,bytes32,bytes32)"))
}

/// `OrderSettled(uint256,bytes32,bytes32,bytes32)` — order settlement complete.
pub fn order_settled_topic() -> H256 {
    H256::from(keccak_hash(
        b"OrderSettled(uint256,bytes32,bytes32,bytes32)",
    ))
}

// ── Log generation functions ────────────────────────────────────

/// Generate a `NoteStateChange(bytes32 indexed note, State state)` log.
///
/// - `topics[0]` = event signature hash
/// - `topics[1]` = `note` (indexed bytes32)
/// - `data` = `state` (uint8, ABI-encoded as 32-byte word)
pub fn note_state_change_log(contract: Address, note_hash: H256, state: U256) -> Log {
    let mut data = [0u8; 32];
    data[31] = state.low_u64() as u8;
    Log {
        address: contract,
        topics: vec![note_state_change_topic(), note_hash],
        data: Bytes::copy_from_slice(&data),
    }
}

/// Generate an `OrderTaken(uint256 indexed orderId, bytes32 takerNoteToMaker, bytes32 parentNote)` log.
///
/// - `topics[0]` = event signature hash
/// - `topics[1]` = `orderId` (indexed uint256)
/// - `data` = `takerNoteToMaker(32) + parentNote(32)` (64 bytes)
pub fn order_taken_log(contract: Address, order_id: U256, taker_note: H256, parent: H256) -> Log {
    let mut data = vec![0u8; 64];
    data[0..32].copy_from_slice(taker_note.as_bytes());
    data[32..64].copy_from_slice(parent.as_bytes());
    Log {
        address: contract,
        topics: vec![order_taken_topic(), u256_to_h256(order_id)],
        data: Bytes::from(data),
    }
}

/// Generate an `OrderSettled(uint256 indexed orderId, bytes32 rewardNote, bytes32 paymentNote, bytes32 changeNote)` log.
///
/// - `topics[0]` = event signature hash
/// - `topics[1]` = `orderId` (indexed uint256)
/// - `data` = `rewardNote(32) + paymentNote(32) + changeNote(32)` (96 bytes)
pub fn order_settled_log(
    contract: Address,
    order_id: U256,
    reward: H256,
    payment: H256,
    change: H256,
) -> Log {
    let mut data = vec![0u8; 96];
    data[0..32].copy_from_slice(reward.as_bytes());
    data[32..64].copy_from_slice(payment.as_bytes());
    data[64..96].copy_from_slice(change.as_bytes());
    Log {
        address: contract,
        topics: vec![order_settled_topic(), u256_to_h256(order_id)],
        data: Bytes::from(data),
    }
}

// ── Helpers ─────────────────────────────────────────────────────

fn u256_to_h256(u: U256) -> H256 {
    H256::from(u.to_big_endian())
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::H160;

    fn dex_addr() -> Address {
        H160([0xDE; 20])
    }

    #[test]
    fn note_state_change_log_format() {
        let note = H256::from_low_u64_be(42);
        let state = U256::from(1); // Valid
        let log = note_state_change_log(dex_addr(), note, state);

        assert_eq!(log.address, dex_addr());
        assert_eq!(log.topics.len(), 2);
        assert_eq!(log.topics[0], note_state_change_topic());
        assert_eq!(log.topics[1], note);
        assert_eq!(log.data.len(), 32);
        assert_eq!(log.data[31], 1); // State.Valid = 1
    }

    #[test]
    fn order_taken_log_format() {
        let order_id = U256::from(5);
        let taker = H256::from_low_u64_be(100);
        let parent = H256::from_low_u64_be(200);
        let log = order_taken_log(dex_addr(), order_id, taker, parent);

        assert_eq!(log.topics.len(), 2);
        assert_eq!(log.topics[0], order_taken_topic());
        assert_eq!(log.data.len(), 64);
        assert_eq!(&log.data[0..32], taker.as_bytes());
        assert_eq!(&log.data[32..64], parent.as_bytes());
    }

    #[test]
    fn order_settled_log_format() {
        let order_id = U256::from(3);
        let reward = H256::from_low_u64_be(10);
        let payment = H256::from_low_u64_be(20);
        let change = H256::from_low_u64_be(30);
        let log = order_settled_log(dex_addr(), order_id, reward, payment, change);

        assert_eq!(log.topics.len(), 2);
        assert_eq!(log.topics[0], order_settled_topic());
        assert_eq!(log.data.len(), 96);
        assert_eq!(&log.data[0..32], reward.as_bytes());
        assert_eq!(&log.data[32..64], payment.as_bytes());
        assert_eq!(&log.data[64..96], change.as_bytes());
    }
}
