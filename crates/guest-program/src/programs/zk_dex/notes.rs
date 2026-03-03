//! Note operations for the ZkDex circuit.
//!
//! Implements state transitions for note-based operations:
//! - **mint**: Create a new note (deposit ETH/DAI into private pool)
//! - **spend**: Spend old notes to create new notes (private transfer)
//! - **liquidate**: Destroy a note and withdraw ETH/DAI
//! - **convertNote**: Convert a smart note to a regular note

use ethrex_common::{Address, H256, U256};

use crate::common::app_execution::{AppCircuitError, OperationResult};
use crate::common::app_state::AppState;

use super::storage::{note_state_slot, write_encrypted_note};

// ── Note state constants ────────────────────────────────────────

/// Note does not exist or has been invalidated.
pub const NOTE_INVALID: U256 = U256([0, 0, 0, 0]);

/// Note exists and is usable.
pub const NOTE_VALID: U256 = U256([1, 0, 0, 0]);

/// Note is locked in an active order (being traded).
pub const NOTE_TRADING: U256 = U256([2, 0, 0, 0]);

/// Note has been spent or liquidated.
pub const NOTE_SPENT: U256 = U256([3, 0, 0, 0]);

/// Poseidon(0,0,0,0,0,0,0) — sentinel for empty note positions in `spend`.
pub const EMPTY_NOTE_HASH: H256 = H256([
    0x0a, 0x47, 0xea, 0xd7, 0x4d, 0xa5, 0x37, 0x2e, 0x7d, 0x25, 0x98, 0xe4, 0xf9, 0x3c, 0x38, 0x9b,
    0xf0, 0x3e, 0x83, 0x30, 0x21, 0x9f, 0x8b, 0xf1, 0xe4, 0x9b, 0x36, 0x2f, 0x73, 0x49, 0x1a, 0x26,
]);

/// ETH token type identifier.
pub const ETH_TOKEN_TYPE: u64 = 0;

// ── Mint ────────────────────────────────────────────────────────

/// Execute a `mint` operation.
///
/// Creates a new note with `State.Valid` and stores its encrypted data.
/// ETH value transfer (msg.value → contract) is handled by `app_execution.rs`.
///
/// ## Params layout
/// - `[0..32]`  — noteHash (bytes32)
/// - `[32..64]` — value (uint256)
/// - `[64..96]` — tokenType (uint256)
/// - `[96..]`   — encryptedNote (bytes)
pub fn execute_mint(
    state: &mut AppState,
    contract: Address,
    params: &[u8],
) -> Result<OperationResult, AppCircuitError> {
    if params.len() < 96 {
        return Err(AppCircuitError::InvalidParams(
            "mint params too short".into(),
        ));
    }

    let note_hash = H256::from_slice(&params[0..32]);
    let encrypted_note = &params[96..];

    // 1. notes[noteHash] = State.Valid
    state.set_storage(contract, note_state_slot(note_hash), NOTE_VALID)?;

    // 2. encryptedNotes[noteHash] = encryptedNote
    write_encrypted_note(state, contract, note_hash, encrypted_note)?;

    // ETH/DAI value transfer is handled by app_execution.rs (tx.value).

    Ok(OperationResult {
        success: true,
        data: vec![],
    })
}

// ── Spend ───────────────────────────────────────────────────────

/// Execute a `spend` operation.
///
/// Spends up to 2 old notes and creates up to 2 new notes.
/// Empty note positions (EMPTY_NOTE_HASH) are skipped.
///
/// ## Params layout
/// - `[0..32]`    — oldNote0Hash (bytes32)
/// - `[32..64]`   — oldNote1Hash (bytes32)
/// - `[64..96]`   — newNoteHash (bytes32)
/// - `[96..128]`  — changeNoteHash (bytes32)
/// - `[128..132]` — enc1 length (u32 big-endian)
/// - `[132..132+enc1_len]` — encryptedNote1 (bytes)
/// - `[132+enc1_len..]`    — encryptedNote2 (bytes)
pub fn execute_spend(
    state: &mut AppState,
    contract: Address,
    params: &[u8],
) -> Result<OperationResult, AppCircuitError> {
    if params.len() < 132 {
        return Err(AppCircuitError::InvalidParams(
            "spend params too short".into(),
        ));
    }

    let old_note0 = H256::from_slice(&params[0..32]);
    let old_note1 = H256::from_slice(&params[32..64]);
    let new_note = H256::from_slice(&params[64..96]);
    let change_note = H256::from_slice(&params[96..128]);

    // Extract encrypted notes with length prefix.
    let enc1_len =
        u32::from_be_bytes([params[128], params[129], params[130], params[131]]) as usize;
    if params.len() < 132 + enc1_len {
        return Err(AppCircuitError::InvalidParams(
            "spend enc1 data too short".into(),
        ));
    }
    let enc1 = &params[132..132 + enc1_len];
    let enc2 = &params[132 + enc1_len..];

    // Spend old notes (skip EMPTY_NOTE_HASH).
    if old_note0 != EMPTY_NOTE_HASH {
        state.set_storage(contract, note_state_slot(old_note0), NOTE_SPENT)?;
    }
    if old_note1 != EMPTY_NOTE_HASH {
        state.set_storage(contract, note_state_slot(old_note1), NOTE_SPENT)?;
    }

    // Create new notes (skip EMPTY_NOTE_HASH).
    if new_note != EMPTY_NOTE_HASH {
        state.set_storage(contract, note_state_slot(new_note), NOTE_VALID)?;
        write_encrypted_note(state, contract, new_note, enc1)?;
    }
    if change_note != EMPTY_NOTE_HASH {
        state.set_storage(contract, note_state_slot(change_note), NOTE_VALID)?;
        write_encrypted_note(state, contract, change_note, enc2)?;
    }

    Ok(OperationResult {
        success: true,
        data: vec![],
    })
}

// ── Liquidate ───────────────────────────────────────────────────

/// Execute a `liquidate` operation.
///
/// Destroys a note and transfers value from the contract to a recipient.
/// Currently supports ETH only; DAI would require the DAI contract's storage.
///
/// ## Params layout
/// - `[0..32]`   — to (address, ABI-encoded 32-byte word)
/// - `[32..64]`  — noteHash (bytes32)
/// - `[64..96]`  — value (uint256)
/// - `[96..128]` — tokenType (uint256)
pub fn execute_liquidate(
    state: &mut AppState,
    contract: Address,
    params: &[u8],
) -> Result<OperationResult, AppCircuitError> {
    if params.len() < 128 {
        return Err(AppCircuitError::InvalidParams(
            "liquidate params too short".into(),
        ));
    }

    let to = Address::from_slice(&params[12..32]);
    let note_hash = H256::from_slice(&params[32..64]);
    let value = U256::from_big_endian(&params[64..96]);
    let token_type = U256::from_big_endian(&params[96..128]).low_u64();

    // 1. notes[noteHash] = State.Spent
    state.set_storage(contract, note_state_slot(note_hash), NOTE_SPENT)?;

    // 2. Transfer value from contract to recipient (ETH only).
    if token_type == ETH_TOKEN_TYPE && !value.is_zero() {
        state.debit_balance(contract, value)?;
        state.credit_balance(to, value)?;
    }
    // DAI transfers would require modifying the DAI contract's storage,
    // which is outside the scope of the DEX circuit.

    Ok(OperationResult {
        success: true,
        data: vec![],
    })
}

// ── ConvertNote ─────────────────────────────────────────────────

/// Execute a `convertNote` operation.
///
/// Invalidates a smart note and creates a new regular note.
///
/// ## Params layout
/// - `[0..32]`  — smartNote (bytes32, input[1])
/// - `[32..64]` — originalNote (bytes32, input[2], unused for state)
/// - `[64..96]` — newNote (bytes32, input[3])
/// - `[96..]`   — encryptedNote (bytes)
pub fn execute_convert_note(
    state: &mut AppState,
    contract: Address,
    params: &[u8],
) -> Result<OperationResult, AppCircuitError> {
    if params.len() < 96 {
        return Err(AppCircuitError::InvalidParams(
            "convertNote params too short".into(),
        ));
    }

    let smart_note = H256::from_slice(&params[0..32]);
    // originalNote (params[32..64]) is not used for state changes.
    let new_note = H256::from_slice(&params[64..96]);
    let encrypted_note = &params[96..];

    // 1. notes[smartNote] = State.Invalid
    state.set_storage(contract, note_state_slot(smart_note), NOTE_INVALID)?;

    // 2. notes[newNote] = State.Valid
    state.set_storage(contract, note_state_slot(new_note), NOTE_VALID)?;

    // 3. encryptedNotes[newNote] = encryptedNote
    write_encrypted_note(state, contract, new_note, encrypted_note)?;

    Ok(OperationResult {
        success: true,
        data: vec![],
    })
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::app_types::{AccountProof, StorageProof};
    use crate::programs::zk_dex::storage::{
        encrypted_note_data_start, encrypted_note_length_slot, encrypted_note_slots,
    };
    use ethrex_common::H160;

    fn dex_address() -> Address {
        H160([0xDE; 20])
    }

    fn make_state_with_note_slots(note_hashes: &[H256]) -> AppState {
        let contract = dex_address();
        let account_proofs = vec![AccountProof {
            address: contract,
            nonce: 0,
            balance: U256::from(1_000_000),
            storage_root: H256::zero(),
            code_hash: H256::zero(),
            proof: vec![],
        }];

        let mut storage_proofs = Vec::new();
        for hash in note_hashes {
            // Note state slot (initially 0 = Invalid)
            storage_proofs.push(StorageProof {
                address: contract,
                slot: note_state_slot(*hash),
                value: U256::zero(),
                account_proof: vec![],
                storage_proof: vec![],
            });
            // Encrypted note slots
            for slot in encrypted_note_slots(*hash, 64) {
                storage_proofs.push(StorageProof {
                    address: contract,
                    slot,
                    value: U256::zero(),
                    account_proof: vec![],
                    storage_proof: vec![],
                });
            }
        }

        AppState::from_proofs(H256::zero(), account_proofs, storage_proofs)
    }

    #[test]
    fn execute_mint_sets_note_valid() {
        let note_hash = H256::from_low_u64_be(1);
        let mut state = make_state_with_note_slots(&[note_hash]);

        let mut params = Vec::new();
        params.extend_from_slice(note_hash.as_bytes()); // noteHash
        params.extend_from_slice(&U256::from(100).to_big_endian()); // value
        params.extend_from_slice(&U256::zero().to_big_endian()); // tokenType = ETH
        params.extend_from_slice(&[0xAB; 64]); // encryptedNote (64 bytes)

        let result = execute_mint(&mut state, dex_address(), &params).unwrap();
        assert!(result.success);

        // Check note is Valid.
        let slot = note_state_slot(note_hash);
        assert_eq!(state.get_storage(dex_address(), slot).unwrap(), NOTE_VALID);

        // Check encrypted note length slot.
        let len_slot = encrypted_note_length_slot(note_hash);
        let stored_len = state.get_storage(dex_address(), len_slot).unwrap();
        // Long format: 64 * 2 + 1 = 129
        assert_eq!(stored_len, U256::from(129));
    }

    #[test]
    fn execute_spend_spends_and_creates() {
        let old0 = H256::from_low_u64_be(10);
        let old1 = EMPTY_NOTE_HASH; // should be skipped
        let new_note = H256::from_low_u64_be(20);
        let change = H256::from_low_u64_be(30);

        let mut state = make_state_with_note_slots(&[old0, new_note, change]);
        // Set old0 to Valid first.
        state
            .set_storage(dex_address(), note_state_slot(old0), NOTE_VALID)
            .unwrap();

        let mut params = Vec::new();
        params.extend_from_slice(old0.as_bytes());
        params.extend_from_slice(old1.as_bytes());
        params.extend_from_slice(new_note.as_bytes());
        params.extend_from_slice(change.as_bytes());

        let enc1 = vec![0x11; 64]; // encryptedNote1
        let enc2 = vec![0x22; 64]; // encryptedNote2
        params.extend_from_slice(&(enc1.len() as u32).to_be_bytes());
        params.extend_from_slice(&enc1);
        params.extend_from_slice(&enc2);

        let result = execute_spend(&mut state, dex_address(), &params).unwrap();
        assert!(result.success);

        // old0 should be Spent.
        assert_eq!(
            state
                .get_storage(dex_address(), note_state_slot(old0))
                .unwrap(),
            NOTE_SPENT
        );
        // new_note and change should be Valid.
        assert_eq!(
            state
                .get_storage(dex_address(), note_state_slot(new_note))
                .unwrap(),
            NOTE_VALID
        );
        assert_eq!(
            state
                .get_storage(dex_address(), note_state_slot(change))
                .unwrap(),
            NOTE_VALID
        );
    }

    #[test]
    fn execute_liquidate_spends_and_transfers_eth() {
        let note_hash = H256::from_low_u64_be(42);
        let recipient = H160([0x01; 20]);
        let contract = dex_address();

        let account_proofs = vec![
            AccountProof {
                address: contract,
                nonce: 0,
                balance: U256::from(10_000),
                storage_root: H256::zero(),
                code_hash: H256::zero(),
                proof: vec![],
            },
            AccountProof {
                address: recipient,
                nonce: 0,
                balance: U256::zero(),
                storage_root: H256::zero(),
                code_hash: H256::zero(),
                proof: vec![],
            },
        ];
        let storage_proofs = vec![StorageProof {
            address: contract,
            slot: note_state_slot(note_hash),
            value: NOTE_VALID,
            account_proof: vec![],
            storage_proof: vec![],
        }];
        let mut state = AppState::from_proofs(H256::zero(), account_proofs, storage_proofs);

        let mut params = Vec::new();
        // ABI-encoded address (32 bytes, last 20 are address)
        let mut addr_word = [0u8; 32];
        addr_word[12..32].copy_from_slice(recipient.as_bytes());
        params.extend_from_slice(&addr_word);
        params.extend_from_slice(note_hash.as_bytes()); // noteHash
        params.extend_from_slice(&U256::from(500).to_big_endian()); // value
        params.extend_from_slice(&U256::zero().to_big_endian()); // tokenType = ETH

        let result = execute_liquidate(&mut state, contract, &params).unwrap();
        assert!(result.success);

        // Note should be Spent.
        assert_eq!(
            state
                .get_storage(contract, note_state_slot(note_hash))
                .unwrap(),
            NOTE_SPENT
        );
        // Contract balance should decrease.
        assert_eq!(state.get_balance(contract).unwrap(), U256::from(9_500));
        // Recipient should receive ETH.
        assert_eq!(state.get_balance(recipient).unwrap(), U256::from(500));
    }

    #[test]
    fn execute_convert_note_invalidates_and_creates() {
        let smart = H256::from_low_u64_be(100);
        let new_note = H256::from_low_u64_be(200);
        let mut state = make_state_with_note_slots(&[smart, new_note]);

        // Set smart note to Valid first.
        state
            .set_storage(dex_address(), note_state_slot(smart), NOTE_VALID)
            .unwrap();

        let mut params = Vec::new();
        params.extend_from_slice(smart.as_bytes()); // smartNote
        params.extend_from_slice(&H256::from_low_u64_be(50).0); // originalNote (unused)
        params.extend_from_slice(new_note.as_bytes()); // newNote
        params.extend_from_slice(&[0xCC; 64]); // encryptedNote

        let result = execute_convert_note(&mut state, dex_address(), &params).unwrap();
        assert!(result.success);

        assert_eq!(
            state
                .get_storage(dex_address(), note_state_slot(smart))
                .unwrap(),
            NOTE_INVALID
        );
        assert_eq!(
            state
                .get_storage(dex_address(), note_state_slot(new_note))
                .unwrap(),
            NOTE_VALID
        );
    }
}
