//! DexCircuit — AppCircuit implementation for the ZK-DEX guest program.
//!
//! Supports 8 operation types:
//! - **TokenTransfer** (ERC-20 style token transfer)
//! - **Mint** (deposit into private note)
//! - **Spend** (private transfer via notes)
//! - **Liquidate** (withdraw from note to address)
//! - **ConvertNote** (convert smart note to regular note)
//! - **MakeOrder** (create DEX order)
//! - **TakeOrder** (take DEX order)
//! - **SettleOrder** (settle DEX order)

use bytes::Bytes;
use ethrex_common::types::{Log, Transaction, TxKind};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::keccak::keccak_hash;

use crate::common::app_execution::{AppCircuit, AppCircuitError, AppOperation, OperationResult};
use crate::common::app_state::AppState;

use super::events;
use super::notes::{
    EMPTY_NOTE_HASH, NOTE_INVALID, NOTE_SPENT, NOTE_TRADING, NOTE_VALID, execute_convert_note,
    execute_liquidate, execute_mint, execute_spend,
};
use super::orders::{execute_make_order, execute_settle_order, execute_take_order};

// ── Operation type constants ─────────────────────────────────────

pub const OP_TOKEN_TRANSFER: u8 = 0;
pub const OP_MINT: u8 = 1;
pub const OP_SPEND: u8 = 2;
pub const OP_LIQUIDATE: u8 = 3;
pub const OP_CONVERT_NOTE: u8 = 4;
pub const OP_MAKE_ORDER: u8 = 5;
pub const OP_TAKE_ORDER: u8 = 6;
pub const OP_SETTLE_ORDER: u8 = 7;

// ── Gas cost constants ──────────────────────────────────────────
// These are reference values. Actual gas is derived from the block
// header's gas_used field in app_execution.rs.

pub const TOKEN_TRANSFER_GAS: u64 = 65_000;
pub const MINT_GAS: u64 = 150_000;
pub const SPEND_GAS: u64 = 200_000;
pub const LIQUIDATE_GAS: u64 = 150_000;
pub const CONVERT_NOTE_GAS: u64 = 150_000;
pub const MAKE_ORDER_GAS: u64 = 200_000;
pub const TAKE_ORDER_GAS: u64 = 200_000;
pub const SETTLE_ORDER_GAS: u64 = 300_000;

// ── ABI selectors ───────────────────────────────────────────────

fn selector_from(sig: &[u8]) -> [u8; 4] {
    let h = keccak_hash(sig);
    [h[0], h[1], h[2], h[3]]
}

fn transfer_selector() -> [u8; 4] {
    selector_from(b"transfer(address,address,uint256)")
}

pub fn transfer_selector_bytes() -> [u8; 4] {
    transfer_selector()
}

fn mint_selector() -> [u8; 4] {
    selector_from(b"mint(uint256[2],uint256[2][2],uint256[2],uint256[4],bytes)")
}

pub fn mint_selector_bytes() -> [u8; 4] {
    mint_selector()
}

fn spend_selector() -> [u8; 4] {
    selector_from(b"spend(uint256[2],uint256[2][2],uint256[2],uint256[5],bytes,bytes)")
}

pub fn spend_selector_bytes() -> [u8; 4] {
    spend_selector()
}

fn liquidate_selector() -> [u8; 4] {
    selector_from(b"liquidate(address,uint256[2],uint256[2][2],uint256[2],uint256[4])")
}

pub fn liquidate_selector_bytes() -> [u8; 4] {
    liquidate_selector()
}

fn convert_note_selector() -> [u8; 4] {
    selector_from(b"convertNote(uint256[2],uint256[2][2],uint256[2],uint256[4],bytes)")
}

pub fn convert_note_selector_bytes() -> [u8; 4] {
    convert_note_selector()
}

fn make_order_selector() -> [u8; 4] {
    selector_from(b"makeOrder(uint256,uint256,uint256[2],uint256[2][2],uint256[2],uint256[3])")
}

pub fn make_order_selector_bytes() -> [u8; 4] {
    make_order_selector()
}

fn take_order_selector() -> [u8; 4] {
    selector_from(b"takeOrder(uint256,uint256[2],uint256[2][2],uint256[2],uint256[6],bytes)")
}

pub fn take_order_selector_bytes() -> [u8; 4] {
    take_order_selector()
}

fn settle_order_selector() -> [u8; 4] {
    selector_from(b"settleOrder(uint256,uint256[2],uint256[2][2],uint256[2],uint256[14],bytes)")
}

pub fn settle_order_selector_bytes() -> [u8; 4] {
    settle_order_selector()
}

/// ERC-20 `Transfer(address,address,uint256)` event topic.
fn transfer_event_topic() -> H256 {
    H256::from(keccak_hash(b"Transfer(address,address,uint256)"))
}

// ── DexCircuit ──────────────────────────────────────────────────

/// ZK-DEX circuit that implements [`AppCircuit`].
pub struct DexCircuit {
    pub contract_address: Address,
}

impl AppCircuit for DexCircuit {
    fn classify_tx(&self, tx: &Transaction) -> Result<AppOperation, AppCircuitError> {
        let to = match tx.to() {
            TxKind::Call(addr) => addr,
            TxKind::Create => return Err(AppCircuitError::UnknownTransaction),
        };
        if to != self.contract_address {
            return Err(AppCircuitError::UnknownTransaction);
        }

        let data = tx.data();
        if data.len() < 4 {
            return Err(AppCircuitError::InvalidParams(
                "calldata too short for selector".into(),
            ));
        }

        let sel = &data[..4];

        if sel == transfer_selector() {
            self.parse_transfer(&data)
        } else if sel == mint_selector() {
            self.parse_mint(&data)
        } else if sel == spend_selector() {
            self.parse_spend(&data)
        } else if sel == liquidate_selector() {
            self.parse_liquidate(&data)
        } else if sel == convert_note_selector() {
            self.parse_convert_note(&data)
        } else if sel == make_order_selector() {
            self.parse_make_order(&data)
        } else if sel == take_order_selector() {
            self.parse_take_order(&data)
        } else if sel == settle_order_selector() {
            self.parse_settle_order(&data)
        } else {
            Err(AppCircuitError::UnknownTransaction)
        }
    }

    fn execute_operation(
        &self,
        state: &mut AppState,
        from: Address,
        op: &AppOperation,
    ) -> Result<OperationResult, AppCircuitError> {
        let contract = self.contract_address;
        match op.op_type {
            OP_TOKEN_TRANSFER => self.execute_transfer(state, from, &op.params),
            OP_MINT => execute_mint(state, contract, &op.params),
            OP_SPEND => execute_spend(state, contract, &op.params),
            OP_LIQUIDATE => execute_liquidate(state, contract, &op.params),
            OP_CONVERT_NOTE => execute_convert_note(state, contract, &op.params),
            OP_MAKE_ORDER => execute_make_order(state, contract, &op.params),
            OP_TAKE_ORDER => execute_take_order(state, contract, &op.params),
            OP_SETTLE_ORDER => execute_settle_order(state, contract, &op.params),
            _ => Err(AppCircuitError::InvalidParams(format!(
                "unknown op_type: {}",
                op.op_type
            ))),
        }
    }

    fn gas_cost(&self, op: &AppOperation) -> u64 {
        match op.op_type {
            OP_TOKEN_TRANSFER => TOKEN_TRANSFER_GAS,
            OP_MINT => MINT_GAS,
            OP_SPEND => SPEND_GAS,
            OP_LIQUIDATE => LIQUIDATE_GAS,
            OP_CONVERT_NOTE => CONVERT_NOTE_GAS,
            OP_MAKE_ORDER => MAKE_ORDER_GAS,
            OP_TAKE_ORDER => TAKE_ORDER_GAS,
            OP_SETTLE_ORDER => SETTLE_ORDER_GAS,
            _ => 0,
        }
    }

    fn generate_logs(
        &self,
        from: Address,
        op: &AppOperation,
        result: &OperationResult,
    ) -> Vec<Log> {
        if !result.success {
            return vec![];
        }
        let c = self.contract_address;
        match op.op_type {
            OP_TOKEN_TRANSFER => self.generate_transfer_logs(from, &op.params),
            OP_MINT => generate_mint_logs(c, &op.params),
            OP_SPEND => generate_spend_logs(c, &op.params),
            OP_LIQUIDATE => generate_liquidate_logs(c, &op.params),
            OP_CONVERT_NOTE => generate_convert_note_logs(c, &op.params),
            OP_MAKE_ORDER => generate_make_order_logs(c, &op.params),
            OP_TAKE_ORDER => generate_take_order_logs(c, &op.params),
            OP_SETTLE_ORDER => generate_settle_order_logs(c, &op.params, &result.data),
            _ => vec![],
        }
    }
}

// ── Transfer (existing) ─────────────────────────────────────────

impl DexCircuit {
    fn parse_transfer(&self, data: &[u8]) -> Result<AppOperation, AppCircuitError> {
        if data.len() < 4 + 96 {
            return Err(AppCircuitError::InvalidParams(
                "calldata too short for transfer params".into(),
            ));
        }
        Ok(AppOperation {
            op_type: OP_TOKEN_TRANSFER,
            params: data[4..4 + 96].to_vec(),
        })
    }

    fn execute_transfer(
        &self,
        state: &mut AppState,
        from: Address,
        params: &[u8],
    ) -> Result<OperationResult, AppCircuitError> {
        if params.len() < 96 {
            return Err(AppCircuitError::InvalidParams(
                "transfer params too short".into(),
            ));
        }

        let to = address_from_abi_word(&params[0..32]);
        let token = address_from_abi_word(&params[32..64]);
        let amount = U256::from_big_endian(&params[64..96]);

        if amount.is_zero() {
            return Ok(OperationResult {
                success: true,
                data: vec![],
            });
        }

        let from_slot = balance_storage_slot(token, from);
        let from_balance = state.get_storage(self.contract_address, from_slot)?;

        if from_balance < amount {
            return Ok(OperationResult {
                success: false,
                data: vec![],
            });
        }

        state.set_storage(self.contract_address, from_slot, from_balance - amount)?;

        let to_slot = balance_storage_slot(token, to);
        let to_balance = state.get_storage(self.contract_address, to_slot)?;
        state.set_storage(self.contract_address, to_slot, to_balance + amount)?;

        Ok(OperationResult {
            success: true,
            data: vec![],
        })
    }

    fn generate_transfer_logs(&self, from: Address, params: &[u8]) -> Vec<Log> {
        if params.len() < 96 {
            return vec![];
        }
        let to = address_from_abi_word(&params[0..32]);
        let amount_bytes = &params[64..96];
        vec![Log {
            address: self.contract_address,
            topics: vec![
                transfer_event_topic(),
                address_to_h256(from),
                address_to_h256(to),
            ],
            data: Bytes::copy_from_slice(amount_bytes),
        }]
    }
}

// ── Calldata parsing ────────────────────────────────────────────

impl DexCircuit {
    /// Parse `mint(uint256[2],uint256[2][2],uint256[2],uint256[4],bytes)`
    ///
    /// Calldata layout:
    /// - [4..260]   Groth16 proof (ignored)
    /// - [260..388]  input[4]: [output, noteHash, value, tokenType]
    /// - [388..420]  offset for encryptedNote
    /// - [at offset] length(32) + encryptedNote data
    ///
    /// Params: noteHash(32) + value(32) + tokenType(32) + encryptedNote(var)
    fn parse_mint(&self, data: &[u8]) -> Result<AppOperation, AppCircuitError> {
        // Minimum: selector(4) + proof(256) + input(128) + offset(32) = 420
        if data.len() < 420 {
            return Err(AppCircuitError::InvalidParams(
                "calldata too short for mint".into(),
            ));
        }

        let note_hash = &data[292..324]; // input[1]
        let value = &data[324..356]; // input[2]
        let token_type = &data[356..388]; // input[3]
        let encrypted_note = extract_abi_bytes(data, 388)?;

        let mut params = Vec::with_capacity(96 + encrypted_note.len());
        params.extend_from_slice(note_hash);
        params.extend_from_slice(value);
        params.extend_from_slice(token_type);
        params.extend_from_slice(&encrypted_note);

        Ok(AppOperation {
            op_type: OP_MINT,
            params,
        })
    }

    /// Parse `spend(uint256[2],uint256[2][2],uint256[2],uint256[5],bytes,bytes)`
    ///
    /// Calldata layout:
    /// - [4..260]    Groth16 proof (ignored)
    /// - [260..420]  input[5]: [output, oldNote0, oldNote1, newNote, changeNote]
    /// - [420..452]  offset for encryptedNote1
    /// - [452..484]  offset for encryptedNote2
    ///
    /// Params: oldNote0(32) + oldNote1(32) + newNote(32) + changeNote(32)
    ///       + enc1_len(4 BE) + enc1(var) + enc2(var)
    fn parse_spend(&self, data: &[u8]) -> Result<AppOperation, AppCircuitError> {
        // Minimum: selector(4) + proof(256) + input(160) + 2*offset(64) = 484
        if data.len() < 484 {
            return Err(AppCircuitError::InvalidParams(
                "calldata too short for spend".into(),
            ));
        }

        let old_note0 = &data[292..324]; // input[1]
        let old_note1 = &data[324..356]; // input[2]
        let new_note = &data[356..388]; // input[3]
        let change_note = &data[388..420]; // input[4]
        let enc1 = extract_abi_bytes(data, 420)?;
        let enc2 = extract_abi_bytes(data, 452)?;

        let mut params = Vec::with_capacity(132 + enc1.len() + enc2.len());
        params.extend_from_slice(old_note0);
        params.extend_from_slice(old_note1);
        params.extend_from_slice(new_note);
        params.extend_from_slice(change_note);
        params.extend_from_slice(&(enc1.len() as u32).to_be_bytes());
        params.extend_from_slice(&enc1);
        params.extend_from_slice(&enc2);

        Ok(AppOperation {
            op_type: OP_SPEND,
            params,
        })
    }

    /// Parse `liquidate(address,uint256[2],uint256[2][2],uint256[2],uint256[4])`
    ///
    /// Calldata layout (all static):
    /// - [4..36]     to (address word)
    /// - [36..292]   Groth16 proof (ignored)
    /// - [292..420]  input[4]: [output, noteHash, value, tokenType]
    ///
    /// Params: to_word(32) + noteHash(32) + value(32) + tokenType(32) = 128
    fn parse_liquidate(&self, data: &[u8]) -> Result<AppOperation, AppCircuitError> {
        // Minimum: selector(4) + to(32) + proof(256) + input(128) = 420
        if data.len() < 420 {
            return Err(AppCircuitError::InvalidParams(
                "calldata too short for liquidate".into(),
            ));
        }

        let to_word = &data[4..36];
        let note_hash = &data[324..356]; // input[1]
        let value = &data[356..388]; // input[2]
        let token_type = &data[388..420]; // input[3]

        let mut params = Vec::with_capacity(128);
        params.extend_from_slice(to_word);
        params.extend_from_slice(note_hash);
        params.extend_from_slice(value);
        params.extend_from_slice(token_type);

        Ok(AppOperation {
            op_type: OP_LIQUIDATE,
            params,
        })
    }

    /// Parse `convertNote(uint256[2],uint256[2][2],uint256[2],uint256[4],bytes)`
    ///
    /// Same layout as mint but different input semantics:
    /// input[1] = smartNote, input[2] = originalNote, input[3] = newNote
    ///
    /// Params: smartNote(32) + originalNote(32) + newNote(32) + encryptedNote(var)
    fn parse_convert_note(&self, data: &[u8]) -> Result<AppOperation, AppCircuitError> {
        if data.len() < 420 {
            return Err(AppCircuitError::InvalidParams(
                "calldata too short for convertNote".into(),
            ));
        }

        let smart_note = &data[292..324]; // input[1]
        let original_note = &data[324..356]; // input[2]
        let new_note = &data[356..388]; // input[3]
        let encrypted_note = extract_abi_bytes(data, 388)?;

        let mut params = Vec::with_capacity(96 + encrypted_note.len());
        params.extend_from_slice(smart_note);
        params.extend_from_slice(original_note);
        params.extend_from_slice(new_note);
        params.extend_from_slice(&encrypted_note);

        Ok(AppOperation {
            op_type: OP_CONVERT_NOTE,
            params,
        })
    }

    /// Parse `makeOrder(uint256,uint256,uint256[2],uint256[2][2],uint256[2],uint256[3])`
    ///
    /// Calldata layout (all static):
    /// - [4..36]     targetToken
    /// - [36..68]    price
    /// - [68..324]   Groth16 proof (ignored)
    /// - [324..420]  input[3]: [output, makerNote, sourceToken]
    ///
    /// Params: targetToken(32) + price(32) + makerNote(32) + sourceToken(32) = 128
    fn parse_make_order(&self, data: &[u8]) -> Result<AppOperation, AppCircuitError> {
        // Minimum: selector(4) + 2*uint256(64) + proof(256) + input(96) = 420
        if data.len() < 420 {
            return Err(AppCircuitError::InvalidParams(
                "calldata too short for makeOrder".into(),
            ));
        }

        let target_token = &data[4..36];
        let price = &data[36..68];
        let maker_note = &data[356..388]; // input[1]
        let source_token = &data[388..420]; // input[2]

        let mut params = Vec::with_capacity(128);
        params.extend_from_slice(target_token);
        params.extend_from_slice(price);
        params.extend_from_slice(maker_note);
        params.extend_from_slice(source_token);

        Ok(AppOperation {
            op_type: OP_MAKE_ORDER,
            params,
        })
    }

    /// Parse `takeOrder(uint256,uint256[2],uint256[2][2],uint256[2],uint256[6],bytes)`
    ///
    /// Calldata layout:
    /// - [4..36]     orderId
    /// - [36..292]   Groth16 proof (ignored)
    /// - [292..484]  input[6]: [output, parentNote, parentNoteType,
    ///                          stakeNote, stakeParentHash, stakeNoteType]
    /// - [484..516]  offset for encryptedStakingNote
    ///
    /// Params: orderId(32) + parentNote(32) + stakeNote(32) + encryptedStakingNote(var)
    fn parse_take_order(&self, data: &[u8]) -> Result<AppOperation, AppCircuitError> {
        // Minimum: selector(4) + orderId(32) + proof(256) + input(192) + offset(32) = 516
        if data.len() < 516 {
            return Err(AppCircuitError::InvalidParams(
                "calldata too short for takeOrder".into(),
            ));
        }

        let order_id = &data[4..36];
        let parent_note = &data[324..356]; // input[1]
        let stake_note = &data[388..420]; // input[3]
        let encrypted = extract_abi_bytes(data, 484)?;

        let mut params = Vec::with_capacity(96 + encrypted.len());
        params.extend_from_slice(order_id);
        params.extend_from_slice(parent_note);
        params.extend_from_slice(stake_note);
        params.extend_from_slice(&encrypted);

        Ok(AppOperation {
            op_type: OP_TAKE_ORDER,
            params,
        })
    }

    /// Parse `settleOrder(uint256,uint256[2],uint256[2][2],uint256[2],uint256[14],bytes)`
    ///
    /// Calldata layout:
    /// - [4..36]     orderId
    /// - [36..292]   Groth16 proof (ignored)
    /// - [292..740]  input[14]: [output, makerNote, makerType,
    ///                           takerStake, takerType, rewardNote, rewardParent,
    ///                           rewardType, paymentNote, paymentParent, paymentType,
    ///                           changeNote, changeType, price]
    /// - [740..772]  offset for encDatas
    ///
    /// Params: orderId(32) + rewardNote(32) + paymentNote(32) + changeNote(32) + encDatas(var)
    fn parse_settle_order(&self, data: &[u8]) -> Result<AppOperation, AppCircuitError> {
        // Minimum: selector(4) + orderId(32) + proof(256) + input(448) + offset(32) = 772
        if data.len() < 772 {
            return Err(AppCircuitError::InvalidParams(
                "calldata too short for settleOrder".into(),
            ));
        }

        let order_id = &data[4..36];
        let reward_note = &data[452..484]; // input[5]
        let payment_note = &data[548..580]; // input[8]
        let change_note = &data[644..676]; // input[11]
        let enc_datas = extract_abi_bytes(data, 740)?;

        let mut params = Vec::with_capacity(128 + enc_datas.len());
        params.extend_from_slice(order_id);
        params.extend_from_slice(reward_note);
        params.extend_from_slice(payment_note);
        params.extend_from_slice(change_note);
        params.extend_from_slice(&enc_datas);

        Ok(AppOperation {
            op_type: OP_SETTLE_ORDER,
            params,
        })
    }
}

// ── Log generation ──────────────────────────────────────────────

/// Mint logs: NoteStateChange(noteHash, Valid)
fn generate_mint_logs(contract: Address, params: &[u8]) -> Vec<Log> {
    if params.len() < 32 {
        return vec![];
    }
    let note_hash = H256::from_slice(&params[0..32]);
    vec![events::note_state_change_log(
        contract, note_hash, NOTE_VALID,
    )]
}

/// Spend logs: NoteStateChange for up to 4 notes (conditional on EMPTY_NOTE_HASH).
fn generate_spend_logs(contract: Address, params: &[u8]) -> Vec<Log> {
    if params.len() < 128 {
        return vec![];
    }
    let old0 = H256::from_slice(&params[0..32]);
    let old1 = H256::from_slice(&params[32..64]);
    let new_note = H256::from_slice(&params[64..96]);
    let change = H256::from_slice(&params[96..128]);

    let mut logs = Vec::new();
    if old0 != EMPTY_NOTE_HASH {
        logs.push(events::note_state_change_log(contract, old0, NOTE_SPENT));
    }
    if old1 != EMPTY_NOTE_HASH {
        logs.push(events::note_state_change_log(contract, old1, NOTE_SPENT));
    }
    if new_note != EMPTY_NOTE_HASH {
        logs.push(events::note_state_change_log(
            contract, new_note, NOTE_VALID,
        ));
    }
    if change != EMPTY_NOTE_HASH {
        logs.push(events::note_state_change_log(contract, change, NOTE_VALID));
    }
    logs
}

/// Liquidate logs: NoteStateChange(noteHash, Spent)
fn generate_liquidate_logs(contract: Address, params: &[u8]) -> Vec<Log> {
    if params.len() < 64 {
        return vec![];
    }
    let note_hash = H256::from_slice(&params[32..64]);
    vec![events::note_state_change_log(
        contract, note_hash, NOTE_SPENT,
    )]
}

/// ConvertNote logs: NoteStateChange(smartNote, Invalid) + NoteStateChange(newNote, Valid)
fn generate_convert_note_logs(contract: Address, params: &[u8]) -> Vec<Log> {
    if params.len() < 96 {
        return vec![];
    }
    let smart_note = H256::from_slice(&params[0..32]);
    let new_note = H256::from_slice(&params[64..96]);
    vec![
        events::note_state_change_log(contract, smart_note, NOTE_INVALID),
        events::note_state_change_log(contract, new_note, NOTE_VALID),
    ]
}

/// MakeOrder logs: NoteStateChange(makerNote, Trading)
fn generate_make_order_logs(contract: Address, params: &[u8]) -> Vec<Log> {
    if params.len() < 96 {
        return vec![];
    }
    let maker_note = H256::from_slice(&params[64..96]);
    vec![events::note_state_change_log(
        contract,
        maker_note,
        NOTE_TRADING,
    )]
}

/// TakeOrder logs: NoteStateChange(parentNote, Trading) +
///                 NoteStateChange(stakeNote, Trading) +
///                 OrderTaken(orderId, stakeNote, parentNote)
fn generate_take_order_logs(contract: Address, params: &[u8]) -> Vec<Log> {
    if params.len() < 96 {
        return vec![];
    }
    let order_id = U256::from_big_endian(&params[0..32]);
    let parent_note = H256::from_slice(&params[32..64]);
    let stake_note = H256::from_slice(&params[64..96]);

    vec![
        events::note_state_change_log(contract, parent_note, NOTE_TRADING),
        events::note_state_change_log(contract, stake_note, NOTE_TRADING),
        events::order_taken_log(contract, order_id, stake_note, parent_note),
    ]
}

/// SettleOrder logs: 6 NoteStateChange + OrderSettled.
///
/// `result_data` contains the 3 old note hashes read from order storage:
/// makerNote(32) + parentNote(32) + takerNoteToMaker(32)
fn generate_settle_order_logs(contract: Address, params: &[u8], result_data: &[u8]) -> Vec<Log> {
    if params.len() < 128 || result_data.len() < 96 {
        return vec![];
    }
    let order_id = U256::from_big_endian(&params[0..32]);
    let reward_note = H256::from_slice(&params[32..64]);
    let payment_note = H256::from_slice(&params[64..96]);
    let change_note = H256::from_slice(&params[96..128]);

    let maker_note = H256::from_slice(&result_data[0..32]);
    let parent_note = H256::from_slice(&result_data[32..64]);
    let taker_note = H256::from_slice(&result_data[64..96]);

    vec![
        // 3 new notes → Valid
        events::note_state_change_log(contract, reward_note, NOTE_VALID),
        events::note_state_change_log(contract, payment_note, NOTE_VALID),
        events::note_state_change_log(contract, change_note, NOTE_VALID),
        // 3 old notes → Spent
        events::note_state_change_log(contract, maker_note, NOTE_SPENT),
        events::note_state_change_log(contract, parent_note, NOTE_SPENT),
        events::note_state_change_log(contract, taker_note, NOTE_SPENT),
        // OrderSettled event
        events::order_settled_log(contract, order_id, reward_note, payment_note, change_note),
    ]
}

// ── ABI helpers ─────────────────────────────────────────────────

/// Extract dynamic `bytes` from ABI-encoded calldata.
///
/// `offset_pos` is the byte position of the offset field in the full calldata.
/// The offset value points to the length word relative to the start of the
/// params area (byte 4 in calldata).
fn extract_abi_bytes(data: &[u8], offset_pos: usize) -> Result<Vec<u8>, AppCircuitError> {
    if data.len() < offset_pos + 32 {
        return Err(AppCircuitError::InvalidParams(
            "calldata too short for bytes offset".into(),
        ));
    }
    let offset_u256 = U256::from_big_endian(&data[offset_pos..offset_pos + 32]);
    if offset_u256 > U256::from(data.len()) {
        return Err(AppCircuitError::InvalidParams(
            "bytes offset exceeds calldata length".into(),
        ));
    }
    let offset = offset_u256.low_u64() as usize;
    let abs_pos = 4 + offset;
    if data.len() < abs_pos + 32 {
        return Err(AppCircuitError::InvalidParams(
            "calldata too short for bytes length".into(),
        ));
    }
    let length_u256 = U256::from_big_endian(&data[abs_pos..abs_pos + 32]);
    if length_u256 > U256::from(data.len()) {
        return Err(AppCircuitError::InvalidParams(
            "bytes length exceeds calldata length".into(),
        ));
    }
    let length = length_u256.low_u64() as usize;
    if data.len() < abs_pos + 32 + length {
        return Err(AppCircuitError::InvalidParams(
            "calldata too short for bytes data".into(),
        ));
    }
    Ok(data[abs_pos + 32..abs_pos + 32 + length].to_vec())
}

// ── Storage layout helpers ──────────────────────────────────────

/// Compute the storage slot for `balances[token][user]`.
pub fn balance_storage_slot(token: Address, user: Address) -> H256 {
    let mut inner_preimage = [0u8; 64];
    inner_preimage[12..32].copy_from_slice(token.as_bytes());

    let inner_hash = keccak_hash(inner_preimage);

    let mut outer_preimage = [0u8; 64];
    outer_preimage[12..32].copy_from_slice(user.as_bytes());
    outer_preimage[32..64].copy_from_slice(&inner_hash);

    H256::from(keccak_hash(outer_preimage))
}

fn address_from_abi_word(word: &[u8]) -> Address {
    debug_assert!(word.len() >= 32);
    Address::from_slice(&word[12..32])
}

fn address_to_h256(addr: Address) -> H256 {
    let mut buf = [0u8; 32];
    buf[12..32].copy_from_slice(addr.as_bytes());
    H256::from(buf)
}

/// Build ABI-encoded calldata for `transfer(address,address,uint256)`.
pub fn encode_transfer_calldata(to: Address, token: Address, amount: U256) -> Vec<u8> {
    let mut data = Vec::with_capacity(4 + 96);
    data.extend_from_slice(&transfer_selector());

    let mut word = [0u8; 32];
    word[12..32].copy_from_slice(to.as_bytes());
    data.extend_from_slice(&word);

    let mut word = [0u8; 32];
    word[12..32].copy_from_slice(token.as_bytes());
    data.extend_from_slice(&word);

    data.extend_from_slice(&amount.to_big_endian());
    data
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::app_types::{AccountProof, StorageProof};
    use ethrex_common::H160;
    use ethrex_common::types::EIP1559Transaction;

    fn dex_address() -> Address {
        H160([0xDE; 20])
    }

    fn token_address() -> Address {
        H160([0xAA; 20])
    }

    fn user_a() -> Address {
        H160([0x01; 20])
    }

    fn user_b() -> Address {
        H160([0x02; 20])
    }

    fn make_circuit() -> DexCircuit {
        DexCircuit {
            contract_address: dex_address(),
        }
    }

    fn make_test_tx(to: Address, data: Vec<u8>) -> Transaction {
        Transaction::EIP1559Transaction(EIP1559Transaction {
            to: TxKind::Call(to),
            data: Bytes::from(data),
            ..Default::default()
        })
    }

    /// Build an AppState with the DEX contract account and given storage slots.
    fn make_state_with_balances(token: Address, balances: Vec<(Address, U256)>) -> AppState {
        let contract = dex_address();
        let account_proofs = vec![AccountProof {
            address: contract,
            nonce: 0,
            balance: U256::zero(),
            storage_root: H256::zero(),
            code_hash: H256::zero(),
            proof: vec![],
        }];
        let storage_proofs: Vec<StorageProof> = balances
            .into_iter()
            .map(|(user, balance)| {
                let slot = balance_storage_slot(token, user);
                StorageProof {
                    address: contract,
                    slot,
                    value: balance,
                    account_proof: vec![],
                    storage_proof: vec![],
                }
            })
            .collect();
        AppState::from_proofs(H256::zero(), account_proofs, storage_proofs)
    }

    // ── classify_tx tests ────────────────────────────────────────

    #[test]
    fn classify_valid_transfer_tx() {
        let circuit = make_circuit();
        let calldata = encode_transfer_calldata(user_b(), token_address(), U256::from(100));
        let tx = make_test_tx(dex_address(), calldata);

        let op = circuit.classify_tx(&tx).expect("should classify");
        assert_eq!(op.op_type, OP_TOKEN_TRANSFER);
        assert_eq!(op.params.len(), 96);
    }

    #[test]
    fn classify_wrong_contract_fails() {
        let circuit = make_circuit();
        let calldata = encode_transfer_calldata(user_b(), token_address(), U256::from(100));
        let wrong_addr = H160([0xFF; 20]);
        let tx = make_test_tx(wrong_addr, calldata);

        assert!(circuit.classify_tx(&tx).is_err());
    }

    #[test]
    fn classify_unknown_selector_fails() {
        let circuit = make_circuit();
        let tx = make_test_tx(dex_address(), vec![0xDE, 0xAD, 0xBE, 0xEF]);

        assert!(circuit.classify_tx(&tx).is_err());
    }

    #[test]
    fn classify_short_calldata_fails() {
        let circuit = make_circuit();
        let tx = make_test_tx(dex_address(), vec![0x00, 0x01]);

        assert!(circuit.classify_tx(&tx).is_err());
    }

    #[test]
    fn classify_selector_ok_but_params_too_short() {
        let circuit = make_circuit();
        let mut data = transfer_selector().to_vec();
        data.extend_from_slice(&[0u8; 32]);
        let tx = make_test_tx(dex_address(), data);

        assert!(circuit.classify_tx(&tx).is_err());
    }

    // ── execute_operation tests ──────────────────────────────────

    #[test]
    fn execute_successful_transfer() {
        let circuit = make_circuit();
        let token = token_address();
        let mut state = make_state_with_balances(
            token,
            vec![(user_a(), U256::from(1000)), (user_b(), U256::from(500))],
        );

        let calldata = encode_transfer_calldata(user_b(), token, U256::from(300));
        let tx = make_test_tx(dex_address(), calldata);
        let op = circuit.classify_tx(&tx).unwrap();

        let result = circuit
            .execute_operation(&mut state, user_a(), &op)
            .expect("should succeed");
        assert!(result.success);

        let a_slot = balance_storage_slot(token, user_a());
        let b_slot = balance_storage_slot(token, user_b());
        assert_eq!(
            state.get_storage(dex_address(), a_slot).unwrap(),
            U256::from(700)
        );
        assert_eq!(
            state.get_storage(dex_address(), b_slot).unwrap(),
            U256::from(800)
        );
    }

    #[test]
    fn execute_insufficient_balance() {
        let circuit = make_circuit();
        let token = token_address();
        let mut state = make_state_with_balances(
            token,
            vec![(user_a(), U256::from(50)), (user_b(), U256::from(0))],
        );

        let calldata = encode_transfer_calldata(user_b(), token, U256::from(100));
        let tx = make_test_tx(dex_address(), calldata);
        let op = circuit.classify_tx(&tx).unwrap();

        let result = circuit
            .execute_operation(&mut state, user_a(), &op)
            .expect("should return failure result, not error");
        assert!(!result.success);

        let a_slot = balance_storage_slot(token, user_a());
        assert_eq!(
            state.get_storage(dex_address(), a_slot).unwrap(),
            U256::from(50)
        );
    }

    #[test]
    fn execute_zero_amount_is_noop() {
        let circuit = make_circuit();
        let token = token_address();
        let mut state = make_state_with_balances(
            token,
            vec![(user_a(), U256::from(100)), (user_b(), U256::from(0))],
        );

        let calldata = encode_transfer_calldata(user_b(), token, U256::zero());
        let tx = make_test_tx(dex_address(), calldata);
        let op = circuit.classify_tx(&tx).unwrap();

        let result = circuit
            .execute_operation(&mut state, user_a(), &op)
            .expect("should succeed");
        assert!(result.success);

        let a_slot = balance_storage_slot(token, user_a());
        assert_eq!(
            state.get_storage(dex_address(), a_slot).unwrap(),
            U256::from(100)
        );
    }

    #[test]
    fn execute_self_transfer() {
        let circuit = make_circuit();
        let token = token_address();
        let mut state = make_state_with_balances(token, vec![(user_a(), U256::from(500))]);

        let calldata = encode_transfer_calldata(user_a(), token, U256::from(100));
        let tx = make_test_tx(dex_address(), calldata);
        let op = circuit.classify_tx(&tx).unwrap();

        let result = circuit
            .execute_operation(&mut state, user_a(), &op)
            .expect("should succeed");
        assert!(result.success);

        let a_slot = balance_storage_slot(token, user_a());
        assert_eq!(
            state.get_storage(dex_address(), a_slot).unwrap(),
            U256::from(500)
        );
    }

    // ── gas_cost tests ───────────────────────────────────────────

    #[test]
    fn gas_cost_token_transfer() {
        let circuit = make_circuit();
        let op = AppOperation {
            op_type: OP_TOKEN_TRANSFER,
            params: vec![0; 96],
        };
        assert_eq!(circuit.gas_cost(&op), TOKEN_TRANSFER_GAS);
        assert_eq!(circuit.gas_cost(&op), 65_000);
    }

    // ── generate_logs tests ──────────────────────────────────────

    #[test]
    fn generate_logs_successful_transfer() {
        let circuit = make_circuit();
        let token = token_address();
        let amount = U256::from(42);
        let calldata = encode_transfer_calldata(user_b(), token, amount);
        let params = calldata[4..].to_vec();

        let op = AppOperation {
            op_type: OP_TOKEN_TRANSFER,
            params,
        };
        let result = OperationResult {
            success: true,
            data: vec![],
        };

        let logs = circuit.generate_logs(user_a(), &op, &result);
        assert_eq!(logs.len(), 1);

        let log = &logs[0];
        assert_eq!(log.address, dex_address());
        assert_eq!(log.topics.len(), 3);
        assert_eq!(log.topics[0], transfer_event_topic());
        assert_eq!(log.topics[1], address_to_h256(user_a()));
        assert_eq!(log.topics[2], address_to_h256(user_b()));

        let expected_amount = amount.to_big_endian();
        assert_eq!(log.data.as_ref(), &expected_amount);
    }

    #[test]
    fn generate_logs_failed_transfer_is_empty() {
        let circuit = make_circuit();
        let op = AppOperation {
            op_type: OP_TOKEN_TRANSFER,
            params: vec![0; 96],
        };
        let result = OperationResult {
            success: false,
            data: vec![],
        };

        let logs = circuit.generate_logs(user_a(), &op, &result);
        assert!(logs.is_empty());
    }

    // ── balance_storage_slot tests ───────────────────────────────

    #[test]
    fn balance_slot_is_deterministic() {
        let slot1 = balance_storage_slot(token_address(), user_a());
        let slot2 = balance_storage_slot(token_address(), user_a());
        assert_eq!(slot1, slot2);
    }

    #[test]
    fn balance_slot_differs_for_different_users() {
        let slot_a = balance_storage_slot(token_address(), user_a());
        let slot_b = balance_storage_slot(token_address(), user_b());
        assert_ne!(slot_a, slot_b);
    }

    #[test]
    fn balance_slot_differs_for_different_tokens() {
        let token1 = H160([0xAA; 20]);
        let token2 = H160([0xBB; 20]);
        let slot1 = balance_storage_slot(token1, user_a());
        let slot2 = balance_storage_slot(token2, user_a());
        assert_ne!(slot1, slot2);
    }

    // ── classify_tx tests for new operations ─────────────────────

    #[test]
    fn classify_mint_selector() {
        let circuit = make_circuit();
        // Build mint calldata: selector + proof(256) + input[4](128) + offset(32) + length(32) + data
        let mut data = mint_selector().to_vec();
        data.extend_from_slice(&[0u8; 256]); // proof
        data.extend_from_slice(&[0u8; 128]); // input[4]
        // offset for bytes: points to byte 416 from start of params = 256+128+32 = 416
        let offset_value = U256::from(416);
        data.extend_from_slice(&offset_value.to_big_endian()); // offset
        // length
        let enc_data = vec![0xABu8; 64];
        data.extend_from_slice(&U256::from(enc_data.len()).to_big_endian());
        data.extend_from_slice(&enc_data);
        // Pad to multiple of 32
        let pad = (32 - (enc_data.len() % 32)) % 32;
        data.extend_from_slice(&vec![0u8; pad]);

        let tx = make_test_tx(dex_address(), data);
        let op = circuit.classify_tx(&tx).expect("should classify mint");
        assert_eq!(op.op_type, OP_MINT);
        // params = noteHash(32) + value(32) + tokenType(32) + encryptedNote(64)
        assert_eq!(op.params.len(), 96 + 64);
    }
}
