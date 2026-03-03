//! Storage slot computation for the ZkDex contract.
//!
//! Computes Solidity-compatible storage slots for `notes`, `orders`,
//! `encryptedNotes`, and `balances` mappings based on the ZkDex
//! contract's inheritance-chain storage layout.
//!
//! ## ZkDex Storage Layout
//!
//! ```text
//! ZkDaiBase:
//!   slot 0: development(bool) + dai(address) (packed)
//!   slot 1: requestVerifier (address)
//!   slot 2: encryptedNotes  mapping(bytes32 => bytes)
//!   slot 3: notes            mapping(bytes32 => State)
//!   slot 4: requestedNoteProofs mapping(bytes32 => bytes)
//!   slot 5: verifiedProofs   mapping(bytes32 => bool)
//! MintNotes:    slot 6: mintNoteVerifier
//! SpendNotes:   slot 7: spendNoteVerifier
//! LiquidateNotes: slot 8: liquidateNoteVerifier
//! ZkDex:
//!   slot 9:  convertNoteVerifier
//!   slot 10: makeOrderVerifier
//!   slot 11: takeOrderVerifier
//!   slot 12: settleOrderVerifier
//!   slot 13: orders  Order[]
//! ```
//!
//! > Verified with `forge inspect ZkDex storage-layout`.

use ethrex_common::{Address, H256, U256};
use ethrex_crypto::keccak::keccak_hash;

use crate::common::app_execution::AppCircuitError;
use crate::common::app_state::AppState;

// ── Base slot constants ─────────────────────────────────────────

/// `mapping(bytes32 => bytes) encryptedNotes` at slot 2.
pub const ENCRYPTED_NOTES_SLOT: u64 = 2;

/// `mapping(bytes32 => State) notes` at slot 3.
pub const NOTES_SLOT: u64 = 3;

/// `Order[] orders` at slot 13.
pub const ORDERS_SLOT: u64 = 13;

// ── Slot computation helpers ────────────────────────────────────

/// Compute the storage slot for `notes[noteHash]`.
///
/// Solidity: `mapping(bytes32 => State)` at slot 3.
/// `slot = keccak256(abi.encode(noteHash, 3))`
pub fn note_state_slot(note_hash: H256) -> H256 {
    mapping_slot(note_hash, NOTES_SLOT)
}

/// Compute the storage slot for `orders.length`.
///
/// For dynamic arrays, the length is stored at the base slot itself.
pub fn orders_length_slot() -> H256 {
    H256::from_low_u64_be(ORDERS_SLOT)
}

/// Compute the storage slot for `orders[index].field`.
///
/// For `T[] storage` at slot `p`:
/// - `arr.length` at slot `p`
/// - `arr[i]` starts at `keccak256(p) + i * stride`
///
/// Each `Order` has 7 fields (7 slots per element):
///   0: makerNote, 1: sourceToken, 2: targetToken, 3: price,
///   4: takerNoteToMaker, 5: parentNote, 6: state
pub fn order_field_slot(order_index: U256, field_offset: u64) -> H256 {
    let mut slot_word = [0u8; 32];
    slot_word[24..32].copy_from_slice(&ORDERS_SLOT.to_be_bytes());
    let base = U256::from_big_endian(&keccak_hash(slot_word));

    let slot = base + order_index * U256::from(7) + U256::from(field_offset);
    u256_to_h256(slot)
}

/// Compute the length slot for `encryptedNotes[noteHash]`.
///
/// Solidity: `mapping(bytes32 => bytes)` at slot 2.
/// `slot = keccak256(abi.encode(noteHash, 2))`
pub fn encrypted_note_length_slot(note_hash: H256) -> H256 {
    mapping_slot(note_hash, ENCRYPTED_NOTES_SLOT)
}

/// Compute the data start slot for a long `bytes` value.
///
/// For `bytes` values >= 32 bytes, data is stored starting at
/// `keccak256(length_slot)`.
pub fn encrypted_note_data_start(length_slot: H256) -> H256 {
    H256::from(keccak_hash(length_slot.as_bytes()))
}

// ── Solidity bytes storage encoding ─────────────────────────────

/// Write a `bytes` value to contract storage using Solidity's encoding.
///
/// Solidity stores `bytes` as:
/// - **Short** (length <= 31): data and length packed in one slot.
///   Higher-order bytes = data (left-aligned), lowest byte = `length * 2`.
/// - **Long** (length >= 32): length slot = `length * 2 + 1`,
///   data stored at `keccak256(length_slot) + i` for each 32-byte chunk.
pub fn write_encrypted_note(
    state: &mut AppState,
    contract: Address,
    note_hash: H256,
    data: &[u8],
) -> Result<(), AppCircuitError> {
    let length_slot = encrypted_note_length_slot(note_hash);

    if data.len() <= 31 {
        // Short format: pack data and length into single slot.
        let mut word = [0u8; 32];
        word[..data.len()].copy_from_slice(data);
        word[31] = (data.len() * 2) as u8;
        state.set_storage(contract, length_slot, U256::from_big_endian(&word))?;
    } else {
        // Long format: length slot + data chunks.
        let length_value = U256::from(data.len() * 2 + 1);
        state.set_storage(contract, length_slot, length_value)?;

        let data_start = encrypted_note_data_start(length_slot);
        let data_start_u256 = h256_to_u256(data_start);

        let chunks = data.len().div_ceil(32);
        for i in 0..chunks {
            let start = i * 32;
            let end = std::cmp::min(start + 32, data.len());
            let mut chunk = [0u8; 32];
            chunk[..end - start].copy_from_slice(&data[start..end]);

            let slot = u256_to_h256(data_start_u256 + U256::from(i));
            state.set_storage(contract, slot, U256::from_big_endian(&chunk))?;
        }
    }

    Ok(())
}

/// Collect all storage slots that an encrypted note write will touch.
///
/// Used by the witness analyzer to request the correct storage proofs.
pub fn encrypted_note_slots(note_hash: H256, data_len: usize) -> Vec<H256> {
    let length_slot = encrypted_note_length_slot(note_hash);
    let mut slots = vec![length_slot];

    if data_len >= 32 {
        let data_start = encrypted_note_data_start(length_slot);
        let data_start_u256 = h256_to_u256(data_start);
        let chunks = data_len.div_ceil(32);
        for i in 0..chunks {
            slots.push(u256_to_h256(data_start_u256 + U256::from(i)));
        }
    }

    slots
}

// ── Internal helpers ────────────────────────────────────────────

/// Compute `keccak256(abi.encode(key, slot))` for a `mapping(bytes32 => T)`.
fn mapping_slot(key: H256, base_slot: u64) -> H256 {
    let mut preimage = [0u8; 64];
    preimage[0..32].copy_from_slice(key.as_bytes());
    // base_slot as uint256 big-endian (last 8 bytes of the 32-byte word).
    preimage[56..64].copy_from_slice(&base_slot.to_be_bytes());
    H256::from(keccak_hash(preimage))
}

fn h256_to_u256(h: H256) -> U256 {
    U256::from_big_endian(h.as_bytes())
}

fn u256_to_h256(u: U256) -> H256 {
    H256::from(u.to_big_endian())
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_state_slot_is_deterministic() {
        let hash = H256::from_low_u64_be(42);
        let slot1 = note_state_slot(hash);
        let slot2 = note_state_slot(hash);
        assert_eq!(slot1, slot2);
    }

    #[test]
    fn note_state_slot_differs_for_different_hashes() {
        let hash1 = H256::from_low_u64_be(1);
        let hash2 = H256::from_low_u64_be(2);
        assert_ne!(note_state_slot(hash1), note_state_slot(hash2));
    }

    #[test]
    fn orders_length_slot_is_base_slot() {
        let slot = orders_length_slot();
        assert_eq!(slot, H256::from_low_u64_be(ORDERS_SLOT));
    }

    #[test]
    fn order_field_slot_sequential() {
        // order[0].field0 and order[0].field1 should differ by 1
        let slot0 = order_field_slot(U256::zero(), 0);
        let slot1 = order_field_slot(U256::zero(), 1);
        let diff = h256_to_u256(slot1) - h256_to_u256(slot0);
        assert_eq!(diff, U256::from(1));
    }

    #[test]
    fn order_field_slot_stride_is_seven() {
        // order[0].field0 and order[1].field0 should differ by 7
        let slot_0_0 = order_field_slot(U256::zero(), 0);
        let slot_1_0 = order_field_slot(U256::from(1), 0);
        let diff = h256_to_u256(slot_1_0) - h256_to_u256(slot_0_0);
        assert_eq!(diff, U256::from(7));
    }

    #[test]
    fn encrypted_note_slots_short() {
        let hash = H256::from_low_u64_be(1);
        let slots = encrypted_note_slots(hash, 20); // 20 bytes, short format
        assert_eq!(slots.len(), 1); // only length slot
    }

    #[test]
    fn encrypted_note_slots_long() {
        let hash = H256::from_low_u64_be(1);
        let slots = encrypted_note_slots(hash, 64); // 64 bytes = 2 chunks
        assert_eq!(slots.len(), 3); // length slot + 2 data slots
    }

    #[test]
    fn encrypted_note_slots_exact_boundary() {
        let hash = H256::from_low_u64_be(1);
        // 32 bytes = exactly 1 chunk (long format since len >= 32)
        let slots = encrypted_note_slots(hash, 32);
        assert_eq!(slots.len(), 2); // length slot + 1 data slot
    }

    #[test]
    fn write_encrypted_note_short_encoding() {
        // 31 bytes: short format — data + length packed in one slot.
        use crate::common::app_state::AppState;
        use crate::common::app_types::{AccountProof, StorageProof};

        let contract = ethrex_common::H160([0xDE; 20]);
        let note = H256::from_low_u64_be(42);
        let data = [0xAB_u8; 10]; // 10 bytes = short format

        let length_slot = encrypted_note_length_slot(note);
        let storage_proofs = vec![StorageProof {
            address: contract,
            slot: length_slot,
            value: U256::zero(),
            account_proof: vec![],
            storage_proof: vec![],
        }];
        let account_proofs = vec![AccountProof {
            address: contract,
            nonce: 0,
            balance: U256::zero(),
            storage_root: H256::zero(),
            code_hash: H256::zero(),
            proof: vec![],
        }];
        let mut state = AppState::from_proofs(H256::zero(), account_proofs, storage_proofs);

        write_encrypted_note(&mut state, contract, note, &data).unwrap();

        let stored = state.get_storage(contract, length_slot).unwrap();
        let stored_bytes = stored.to_big_endian();
        // First 10 bytes should be 0xAB, rest zeros except last byte.
        assert_eq!(&stored_bytes[..10], &[0xAB; 10]);
        assert_eq!(&stored_bytes[10..31], &[0u8; 21]);
        // Last byte = length * 2 = 20.
        assert_eq!(stored_bytes[31], 20);
    }

    #[test]
    fn write_encrypted_note_long_encoding() {
        // 32 bytes: long format — length slot + 1 data chunk.
        use crate::common::app_state::AppState;
        use crate::common::app_types::{AccountProof, StorageProof};

        let contract = ethrex_common::H160([0xDE; 20]);
        let note = H256::from_low_u64_be(99);
        let data = [0xCD_u8; 32]; // exactly 32 bytes = long format

        let all_slots = encrypted_note_slots(note, 32);
        assert_eq!(all_slots.len(), 2);

        let mut storage_proofs: Vec<StorageProof> = all_slots
            .iter()
            .map(|s| StorageProof {
                address: contract,
                slot: *s,
                value: U256::zero(),
                account_proof: vec![],
                storage_proof: vec![],
            })
            .collect();
        // Deduplicate (shouldn't happen but be safe).
        storage_proofs.dedup_by_key(|p| p.slot);

        let account_proofs = vec![AccountProof {
            address: contract,
            nonce: 0,
            balance: U256::zero(),
            storage_root: H256::zero(),
            code_hash: H256::zero(),
            proof: vec![],
        }];
        let mut state = AppState::from_proofs(H256::zero(), account_proofs, storage_proofs);

        write_encrypted_note(&mut state, contract, note, &data).unwrap();

        // Length slot should be 2*32+1 = 65.
        let length_val = state.get_storage(contract, all_slots[0]).unwrap();
        assert_eq!(length_val, U256::from(65));

        // Data slot should contain the full 32 bytes.
        let data_val = state.get_storage(contract, all_slots[1]).unwrap();
        assert_eq!(data_val.to_big_endian(), [0xCD_u8; 32]);
    }
}
