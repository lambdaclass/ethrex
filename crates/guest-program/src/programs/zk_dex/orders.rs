//! Order operations for the ZkDex circuit.
//!
//! Implements state transitions for the DEX order book:
//! - **makeOrder**: Create a new order, lock the maker's note
//! - **takeOrder**: Take an order, lock taker's notes
//! - **settleOrder**: Settle an order, create reward/payment/change notes

use ethrex_common::{Address, H256, U256};

use crate::common::app_execution::{AppCircuitError, OperationResult};
use crate::common::app_state::AppState;

use super::notes::{NOTE_SPENT, NOTE_TRADING, NOTE_VALID};
use super::storage::{note_state_slot, order_field_slot, orders_length_slot, write_encrypted_note};

// ── Order struct field offsets ───────────────────────────────────

/// `Order.makerNote` — hash of the maker's note.
pub const ORDER_MAKER_NOTE: u64 = 0;
/// `Order.sourceToken` — token type the maker is offering.
pub const ORDER_SOURCE_TOKEN: u64 = 1;
/// `Order.targetToken` — token type the maker wants to receive.
pub const ORDER_TARGET_TOKEN: u64 = 2;
/// `Order.price` — exchange rate.
pub const ORDER_PRICE: u64 = 3;
/// `Order.takerNoteToMaker` — hash of the note the taker sends to maker.
pub const ORDER_TAKER_NOTE_TO_MAKER: u64 = 4;
/// `Order.parentNote` — hash of the taker's parent note.
pub const ORDER_PARENT_NOTE: u64 = 5;
/// `Order.state` — order lifecycle state.
pub const ORDER_STATE: u64 = 6;

// ── Order state constants ───────────────────────────────────────

/// Order just created, waiting for a taker.
pub const ORDER_STATE_CREATED: U256 = U256([0, 0, 0, 0]);
/// Order taken by a taker.
pub const ORDER_STATE_TAKEN: U256 = U256([1, 0, 0, 0]);
/// Order completed and settled.
pub const ORDER_STATE_SETTLED: U256 = U256([2, 0, 0, 0]);

// ── MakeOrder ───────────────────────────────────────────────────

/// Execute a `makeOrder` operation.
///
/// Creates a new order in the `orders` array and locks the maker's note.
///
/// ## Params layout
/// - `[0..32]`   — targetToken (uint256)
/// - `[32..64]`  — price (uint256)
/// - `[64..96]`  — makerNote (bytes32, input[1])
/// - `[96..128]` — sourceToken (uint256, input[2])
pub fn execute_make_order(
    state: &mut AppState,
    contract: Address,
    params: &[u8],
) -> Result<OperationResult, AppCircuitError> {
    if params.len() < 128 {
        return Err(AppCircuitError::InvalidParams(
            "makeOrder params too short".into(),
        ));
    }

    let target_token = U256::from_big_endian(&params[0..32]);
    let price = U256::from_big_endian(&params[32..64]);
    let maker_note = H256::from_slice(&params[64..96]);
    let source_token = U256::from_big_endian(&params[96..128]);

    // 1. Read current orders.length (= next orderId).
    let order_count = state.get_storage(contract, orders_length_slot())?;

    // 2. Increment orders.length.
    state.set_storage(contract, orders_length_slot(), order_count + U256::from(1))?;

    // 3. Write order fields.
    state.set_storage(
        contract,
        order_field_slot(order_count, ORDER_MAKER_NOTE),
        U256::from_big_endian(maker_note.as_bytes()),
    )?;
    state.set_storage(
        contract,
        order_field_slot(order_count, ORDER_SOURCE_TOKEN),
        source_token,
    )?;
    state.set_storage(
        contract,
        order_field_slot(order_count, ORDER_TARGET_TOKEN),
        target_token,
    )?;
    state.set_storage(contract, order_field_slot(order_count, ORDER_PRICE), price)?;
    state.set_storage(
        contract,
        order_field_slot(order_count, ORDER_STATE),
        ORDER_STATE_CREATED,
    )?;

    // 4. Lock maker note: notes[makerNote] = Trading.
    state.set_storage(contract, note_state_slot(maker_note), NOTE_TRADING)?;

    Ok(OperationResult {
        success: true,
        data: vec![],
    })
}

// ── TakeOrder ───────────────────────────────────────────────────

/// Execute a `takeOrder` operation.
///
/// Locks the taker's notes and updates the order with taker information.
///
/// ## Params layout
/// - `[0..32]`  — orderId (uint256)
/// - `[32..64]` — parentNote (bytes32, input[1])
/// - `[64..96]` — stakeNote (bytes32, input[3], takerNoteToMaker)
/// - `[96..]`   — encryptedStakingNote (bytes)
pub fn execute_take_order(
    state: &mut AppState,
    contract: Address,
    params: &[u8],
) -> Result<OperationResult, AppCircuitError> {
    if params.len() < 96 {
        return Err(AppCircuitError::InvalidParams(
            "takeOrder params too short".into(),
        ));
    }

    let order_id = U256::from_big_endian(&params[0..32]);
    let parent_note = H256::from_slice(&params[32..64]);
    let stake_note = H256::from_slice(&params[64..96]);
    let encrypted_staking_note = &params[96..];

    // 1. Lock notes: notes[parentNote] = Trading, notes[stakeNote] = Trading.
    state.set_storage(contract, note_state_slot(parent_note), NOTE_TRADING)?;
    state.set_storage(contract, note_state_slot(stake_note), NOTE_TRADING)?;

    // 2. Store encrypted staking note.
    write_encrypted_note(state, contract, stake_note, encrypted_staking_note)?;

    // 3. Update order fields.
    state.set_storage(
        contract,
        order_field_slot(order_id, ORDER_TAKER_NOTE_TO_MAKER),
        U256::from_big_endian(stake_note.as_bytes()),
    )?;
    state.set_storage(
        contract,
        order_field_slot(order_id, ORDER_PARENT_NOTE),
        U256::from_big_endian(parent_note.as_bytes()),
    )?;
    state.set_storage(
        contract,
        order_field_slot(order_id, ORDER_STATE),
        ORDER_STATE_TAKEN,
    )?;

    Ok(OperationResult {
        success: true,
        data: vec![],
    })
}

// ── SettleOrder ─────────────────────────────────────────────────

/// Execute a `settleOrder` operation.
///
/// Creates 3 new notes (reward, payment, change), spends 3 old notes
/// (makerNote, parentNote, takerNoteToMaker read from the order), and
/// marks the order as settled.
///
/// ## Params layout
/// - `[0..32]`   — orderId (uint256)
/// - `[32..64]`  — rewardNote (bytes32, input[5])
/// - `[64..96]`  — paymentNote (bytes32, input[8])
/// - `[96..128]` — changeNote (bytes32, input[11])
/// - `[128..]`   — encDatas (RLP-encoded: [encReward, encPayment, encChange])
///
/// ## Result data
/// - `[0..32]`  — makerNote (read from order storage)
/// - `[32..64]` — parentNote (read from order storage)
/// - `[64..96]` — takerNoteToMaker (read from order storage)
pub fn execute_settle_order(
    state: &mut AppState,
    contract: Address,
    params: &[u8],
) -> Result<OperationResult, AppCircuitError> {
    if params.len() < 128 {
        return Err(AppCircuitError::InvalidParams(
            "settleOrder params too short".into(),
        ));
    }

    let order_id = U256::from_big_endian(&params[0..32]);
    let reward_note = H256::from_slice(&params[32..64]);
    let payment_note = H256::from_slice(&params[64..96]);
    let change_note = H256::from_slice(&params[96..128]);
    let enc_datas = &params[128..];

    // Read existing note hashes from order storage.
    let maker_note_u256 =
        state.get_storage(contract, order_field_slot(order_id, ORDER_MAKER_NOTE))?;
    let parent_note_u256 =
        state.get_storage(contract, order_field_slot(order_id, ORDER_PARENT_NOTE))?;
    let taker_note_u256 = state.get_storage(
        contract,
        order_field_slot(order_id, ORDER_TAKER_NOTE_TO_MAKER),
    )?;

    let maker_note = H256::from(maker_note_u256.to_big_endian());
    let parent_note = H256::from(parent_note_u256.to_big_endian());
    let taker_note_to_maker = H256::from(taker_note_u256.to_big_endian());

    // Decode encrypted notes from RLP-encoded encDatas.
    let enc_notes = decode_enc_datas(enc_datas)?;
    if enc_notes.len() < 3 {
        return Err(AppCircuitError::InvalidParams(
            "settleOrder encDatas must contain 3 encrypted notes".into(),
        ));
    }

    // 1. Create 3 new notes → Valid + encrypted notes.
    state.set_storage(contract, note_state_slot(reward_note), NOTE_VALID)?;
    write_encrypted_note(state, contract, reward_note, &enc_notes[0])?;

    state.set_storage(contract, note_state_slot(payment_note), NOTE_VALID)?;
    write_encrypted_note(state, contract, payment_note, &enc_notes[1])?;

    state.set_storage(contract, note_state_slot(change_note), NOTE_VALID)?;
    write_encrypted_note(state, contract, change_note, &enc_notes[2])?;

    // 2. Spend 3 old notes from the order.
    state.set_storage(contract, note_state_slot(maker_note), NOTE_SPENT)?;
    state.set_storage(contract, note_state_slot(parent_note), NOTE_SPENT)?;
    state.set_storage(contract, note_state_slot(taker_note_to_maker), NOTE_SPENT)?;

    // 3. Update order state to Settled.
    state.set_storage(
        contract,
        order_field_slot(order_id, ORDER_STATE),
        ORDER_STATE_SETTLED,
    )?;

    // Return old note hashes in result data for log generation.
    let mut data = Vec::with_capacity(96);
    data.extend_from_slice(maker_note.as_bytes());
    data.extend_from_slice(parent_note.as_bytes());
    data.extend_from_slice(taker_note_to_maker.as_bytes());

    Ok(OperationResult {
        success: true,
        data,
    })
}

// ── RLP decoding for settleOrder encDatas ───────────────────────

/// Decode RLP-encoded encDatas into a list of encrypted note byte strings.
///
/// The encDatas parameter in `settleOrder` is RLP-encoded as a list of 3
/// byte strings: `[encryptedRewardNote, encryptedPaymentNote, encryptedChangeNote]`.
///
/// Uses a simple RLP list decoder (no external dependency).
fn decode_enc_datas(data: &[u8]) -> Result<Vec<Vec<u8>>, AppCircuitError> {
    if data.is_empty() {
        return Err(AppCircuitError::InvalidParams("encDatas is empty".into()));
    }

    // RLP list prefix.
    let (list_data, _) = decode_rlp_list(data)?;

    // Decode items from the list.
    let mut items = Vec::new();
    let mut offset = 0;
    while offset < list_data.len() {
        let (item, consumed) = decode_rlp_bytes(&list_data[offset..])?;
        items.push(item);
        offset += consumed;
    }

    Ok(items)
}

/// Decode an RLP list prefix, returning the inner data and total consumed bytes.
fn decode_rlp_list(data: &[u8]) -> Result<(&[u8], usize), AppCircuitError> {
    let err = || AppCircuitError::InvalidParams("invalid RLP list encoding".into());

    if data.is_empty() {
        return Err(err());
    }

    let prefix = data[0];
    if prefix <= 0xbf {
        // Not a list prefix.
        return Err(err());
    }

    if prefix <= 0xf7 {
        // Short list: length in prefix byte.
        let len = (prefix - 0xc0) as usize;
        if data.len() < 1 + len {
            return Err(err());
        }
        Ok((&data[1..1 + len], 1 + len))
    } else {
        // Long list: length of length follows.
        let len_bytes = (prefix - 0xf7) as usize;
        if data.len() < 1 + len_bytes {
            return Err(err());
        }
        let mut len = 0usize;
        for &b in &data[1..1 + len_bytes] {
            len = len.checked_shl(8).ok_or_else(err)? | (b as usize);
        }
        if data.len() < 1 + len_bytes + len {
            return Err(err());
        }
        Ok((
            &data[1 + len_bytes..1 + len_bytes + len],
            1 + len_bytes + len,
        ))
    }
}

/// Decode an RLP byte string, returning the data and total consumed bytes.
fn decode_rlp_bytes(data: &[u8]) -> Result<(Vec<u8>, usize), AppCircuitError> {
    let err = || AppCircuitError::InvalidParams("invalid RLP bytes encoding".into());

    if data.is_empty() {
        return Err(err());
    }

    let prefix = data[0];

    if prefix <= 0x7f {
        // Single byte.
        Ok((vec![prefix], 1))
    } else if prefix <= 0xb7 {
        // Short string: length in prefix byte.
        let len = (prefix - 0x80) as usize;
        if data.len() < 1 + len {
            return Err(err());
        }
        Ok((data[1..1 + len].to_vec(), 1 + len))
    } else if prefix <= 0xbf {
        // Long string: length of length follows.
        let len_bytes = (prefix - 0xb7) as usize;
        if data.len() < 1 + len_bytes {
            return Err(err());
        }
        let mut len = 0usize;
        for &b in &data[1..1 + len_bytes] {
            len = len.checked_shl(8).ok_or_else(err)? | (b as usize);
        }
        if data.len() < 1 + len_bytes + len {
            return Err(err());
        }
        Ok((
            data[1 + len_bytes..1 + len_bytes + len].to_vec(),
            1 + len_bytes + len,
        ))
    } else {
        // List prefix — unexpected in byte string context.
        Err(err())
    }
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::app_types::{AccountProof, StorageProof};
    use crate::programs::zk_dex::storage::{ORDERS_SLOT, encrypted_note_slots};
    use ethrex_common::H160;

    fn dex_address() -> Address {
        H160([0xDE; 20])
    }

    /// Build a state with order slots and note slots pre-provisioned.
    fn make_order_state(order_count: u64, note_hashes: &[H256], max_order_index: u64) -> AppState {
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

        // orders.length slot
        storage_proofs.push(StorageProof {
            address: contract,
            slot: orders_length_slot(),
            value: U256::from(order_count),
            account_proof: vec![],
            storage_proof: vec![],
        });

        // Order field slots (7 per order)
        for i in 0..=max_order_index {
            for field in 0..7 {
                storage_proofs.push(StorageProof {
                    address: contract,
                    slot: order_field_slot(U256::from(i), field),
                    value: U256::zero(),
                    account_proof: vec![],
                    storage_proof: vec![],
                });
            }
        }

        // Note state and encrypted note slots
        for hash in note_hashes {
            storage_proofs.push(StorageProof {
                address: contract,
                slot: note_state_slot(*hash),
                value: U256::zero(),
                account_proof: vec![],
                storage_proof: vec![],
            });
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
    fn execute_make_order_creates_order() {
        let maker_note = H256::from_low_u64_be(100);
        let mut state = make_order_state(0, &[maker_note], 0);

        let mut params = Vec::new();
        params.extend_from_slice(&U256::from(1).to_big_endian()); // targetToken
        params.extend_from_slice(&U256::from(50).to_big_endian()); // price
        params.extend_from_slice(maker_note.as_bytes()); // makerNote
        params.extend_from_slice(&U256::from(0).to_big_endian()); // sourceToken

        let result = execute_make_order(&mut state, dex_address(), &params).unwrap();
        assert!(result.success);

        // orders.length should be 1
        assert_eq!(
            state
                .get_storage(dex_address(), orders_length_slot())
                .unwrap(),
            U256::from(1)
        );

        // makerNote should be Trading
        assert_eq!(
            state
                .get_storage(dex_address(), note_state_slot(maker_note))
                .unwrap(),
            NOTE_TRADING
        );

        // Order state should be Created
        assert_eq!(
            state
                .get_storage(dex_address(), order_field_slot(U256::zero(), ORDER_STATE))
                .unwrap(),
            ORDER_STATE_CREATED
        );
    }

    #[test]
    fn execute_take_order_locks_notes() {
        let parent = H256::from_low_u64_be(200);
        let stake = H256::from_low_u64_be(300);
        let mut state = make_order_state(1, &[parent, stake], 0);

        let mut params = Vec::new();
        params.extend_from_slice(&U256::zero().to_big_endian()); // orderId = 0
        params.extend_from_slice(parent.as_bytes()); // parentNote
        params.extend_from_slice(stake.as_bytes()); // stakeNote
        params.extend_from_slice(&[0xAA; 64]); // encryptedStakingNote

        let result = execute_take_order(&mut state, dex_address(), &params).unwrap();
        assert!(result.success);

        assert_eq!(
            state
                .get_storage(dex_address(), note_state_slot(parent))
                .unwrap(),
            NOTE_TRADING
        );
        assert_eq!(
            state
                .get_storage(dex_address(), note_state_slot(stake))
                .unwrap(),
            NOTE_TRADING
        );
        assert_eq!(
            state
                .get_storage(dex_address(), order_field_slot(U256::zero(), ORDER_STATE))
                .unwrap(),
            ORDER_STATE_TAKEN
        );
    }

    #[test]
    fn decode_enc_datas_basic() {
        // RLP encode 3 short byte strings.
        // item1 = [0x01, 0x02] => 0x82, 0x01, 0x02 (3 bytes)
        // item2 = [0x03]       => 0x03             (1 byte, single byte)
        // item3 = [0x04, 0x05] => 0x82, 0x04, 0x05 (3 bytes)
        // list length = 7, prefix = 0xc0 + 7 = 0xc7
        let encoded = vec![0xc7, 0x82, 0x01, 0x02, 0x03, 0x82, 0x04, 0x05];
        let items = decode_enc_datas(&encoded).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], vec![0x01, 0x02]);
        assert_eq!(items[1], vec![0x03]);
        assert_eq!(items[2], vec![0x04, 0x05]);
    }
}
