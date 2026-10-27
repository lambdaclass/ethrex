//! EIP-7906 transaction assertions: `TXTRACE` (0xB6) and `EVENTDATACOPY` (0xB7).
//!
//! The two opcodes expose the executing frame transaction's state diff to its
//! `ASSERT` frames, and halt exceptionally anywhere else. The diff is the net
//! difference between the transaction prestate (`GeneralizedDatabase::tx_prestate`,
//! each account and slot as the transaction first touched it) and the live state at
//! the reading instruction. The tables are recomputed on every read from those two
//! maps and from the logs of the frames that have completed.

use ethrex_common::types::{AccountInfo, FrameMode, Log};
use ethrex_common::{Address, H256, U256};

use crate::db::gen_db::CacheDB;
use crate::errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError};
use crate::gas_cost;
use crate::memory::calculate_memory_size;
use crate::opcode_handlers::OpcodeHandler;
use crate::opcode_handlers::frame_tx::address_to_u256;
use crate::utils::size_offset_to_usize;
use crate::vm::VM;

/// One entry of an account-field table (`balances_changed`, `codes_changed`,
/// `nonces_changed`): `(address, before, after)`.
type FieldChange = (Address, U256, U256);
/// One `slots_changed` entry: `(address, slot_key, value_before, value_after)`.
type SlotChange = (Address, H256, U256, U256);

/// An account-field table: every address the transaction touched whose live value
/// of `field` differs from its prestate value, ascending by address.
///
/// The touched set is the prestate map, not the live cache: the cache can span
/// several transactions, so iterating it would report other transactions'
/// changes. An account enters the prestate map on the same first touch that puts
/// it in the cache, so a prestate entry always has a live one. An account that did
/// not exist reads as the empty account: balance and nonce zero, the empty-code
/// hash.
fn field_changes(
    prestate: &CacheDB,
    current: &CacheDB,
    field: impl Fn(&AccountInfo) -> U256,
) -> Vec<FieldChange> {
    let mut changes: Vec<FieldChange> = prestate
        .iter()
        .filter_map(|(address, account)| {
            let before = field(&account.info);
            let after = current
                .get(address)
                .map_or(before, |live| field(&live.info));
            (after != before).then_some((*address, before, after))
        })
        .collect();
    changes.sort_by_key(|change| change.0);
    changes
}

/// The `slots_changed` table: every slot the transaction touched whose live value
/// differs from its prestate value, ascending by address and then by slot key.
fn slot_changes(prestate: &CacheDB, current: &CacheDB) -> Vec<SlotChange> {
    let mut changes: Vec<SlotChange> = Vec::new();
    for (address, account) in prestate {
        let live = current.get(address);
        for (key, before) in &account.storage {
            let after = live
                .and_then(|live| live.storage.get(key).copied())
                .unwrap_or(*before);
            if after != *before {
                changes.push((*address, *key, *before, after));
            }
        }
    }
    // `H256` orders big-endian, which is the slot key's numerical order.
    changes.sort_by_key(|change| (change.0, change.1));
    changes
}

/// The `events_count` table: the logs of every completed frame, in the order the
/// transaction receipt reports them. A frame that failed, or whose atomic batch was
/// unrolled, has no logs in its result and so contributes none.
fn transaction_events(vm: &VM<'_>) -> Vec<Log> {
    vm.frame_tx_context
        .as_ref()
        .map(|ctx| {
            ctx.frame_results
                .iter()
                .flat_map(|result| result.3.iter().cloned())
                .collect()
        })
        .unwrap_or_default()
}

/// Halts unless the executing frame is an `ASSERT` frame. The check keys on the
/// frame, not the call depth, so the opcodes work anywhere in an `ASSERT` frame's
/// call subtree and nowhere else.
fn require_assert_frame(vm: &VM<'_>) -> Result<(), VMError> {
    let ctx = vm
        .frame_tx_context
        .as_ref()
        .ok_or(ExceptionalHalt::InvalidOpcode)?;
    match ctx.tx.frames.get(ctx.current_frame_index) {
        Some(frame) if frame.execution_mode() == FrameMode::Assert => Ok(()),
        _ => Err(ExceptionalHalt::InvalidOpcode.into()),
    }
}

/// The transaction prestate. `VM::new` installs it for every transaction carrying an
/// `ASSERT` frame, and `require_assert_frame` has established that this is one, so
/// its absence is a broken invariant rather than something the opcodes can observe.
fn tx_prestate<'a>(vm: &'a VM<'_>) -> Result<&'a CacheDB, VMError> {
    vm.db
        .tx_prestate
        .as_ref()
        .ok_or_else(|| InternalError::msg("EIP-7906 opcode without a transaction prestate").into())
}

/// Halts unless an operand the parameter table marks *must be 0* is zero.
fn require_zero(operand: U256) -> Result<(), VMError> {
    if operand.is_zero() {
        Ok(())
    } else {
        Err(ExceptionalHalt::InvalidOpcode.into())
    }
}

/// The table entry at `index`, halting when the index is out of bounds.
fn table_entry<T: Clone>(table: &[T], index: U256) -> Result<T, VMError> {
    usize::try_from(index)
        .ok()
        .and_then(|index| table.get(index).cloned())
        .ok_or_else(|| ExceptionalHalt::InvalidOpcode.into())
}

/// The topic at `position`, halting when the event carries no such topic.
fn event_topic(event: &Log, position: usize) -> Result<U256, VMError> {
    event
        .topics
        .get(position)
        .map(|topic| U256::from_big_endian(topic.as_bytes()))
        .ok_or_else(|| ExceptionalHalt::InvalidOpcode.into())
}

fn h256_to_u256(value: H256) -> U256 {
    U256::from_big_endian(value.as_bytes())
}

/// One field of an account-field table entry. Every state-change table lists its
/// fields in the same order, so param `first` reads the address, `first + 1` the
/// before value and `first + 2` the after value.
fn field_entry(change: FieldChange, param: u8, first: u8) -> U256 {
    let (address, before, after) = change;
    match param.saturating_sub(first) {
        0 => address_to_u256(address),
        1 => before,
        _ => after,
    }
}

/// `TXTRACE` (0xB6): one value of the transaction's state diff, selected by `param`
/// (top of the stack) and, for the per-entry params, an `index` into the table it
/// selects. Costs `WARM_STORAGE_READ_COST` for every param.
pub struct OpTxTraceHandler;
impl OpcodeHandler for OpTxTraceHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [param, index] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TXTRACE)?;
        require_assert_frame(vm)?;

        let prestate = tx_prestate(vm)?;
        let current = &vm.db.current_accounts_state;
        // Code hashes are compared as stored: an EIP-7702 delegation indicator is
        // hashed itself, not followed, and a missing account has the empty-code hash.
        let balances = || field_changes(prestate, current, |info| info.balance);
        let codes = || field_changes(prestate, current, |info| h256_to_u256(info.code_hash));
        let nonces = || field_changes(prestate, current, |info| U256::from(info.nonce));
        let param = u8::try_from(param).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let value = match param {
            0x00 => {
                require_zero(index)?;
                U256::from(balances().len())
            }
            0x01 => {
                require_zero(index)?;
                U256::from(slot_changes(prestate, current).len())
            }
            0x02 => {
                require_zero(index)?;
                U256::from(codes().len())
            }
            0x03 => {
                require_zero(index)?;
                U256::from(nonces().len())
            }
            0x04..=0x06 => field_entry(table_entry(&balances(), index)?, param, 0x04),
            0x07..=0x0A => {
                let (address, key, before, after) =
                    table_entry(&slot_changes(prestate, current), index)?;
                match param {
                    0x07 => address_to_u256(address),
                    0x08 => before,
                    0x09 => after,
                    _ => h256_to_u256(key),
                }
            }
            0x0B..=0x0D => field_entry(table_entry(&codes(), index)?, param, 0x0B),
            0x0E..=0x10 => field_entry(table_entry(&nonces(), index)?, param, 0x0E),
            0x11 => {
                require_zero(index)?;
                U256::from(transaction_events(vm).len())
            }
            0x12..=0x18 => {
                let event = table_entry(&transaction_events(vm), index)?;
                match param {
                    0x12 => address_to_u256(event.address),
                    0x13 => U256::from(event.topics.len()),
                    0x14 => event_topic(&event, 0)?,
                    0x15 => event_topic(&event, 1)?,
                    0x16 => event_topic(&event, 2)?,
                    0x17 => event_topic(&event, 3)?,
                    _ => U256::from(event.data.len()),
                }
            }
            0x19 => {
                require_zero(index)?;
                // The payer's escrow: EIP-8141's `max_cost`, blob fees included,
                // collected at payment approval.
                vm.frame_tx_context
                    .as_ref()
                    .ok_or(ExceptionalHalt::InvalidOpcode)?
                    .fee_budget
                    .max_cost
            }
            0x1A => {
                require_zero(index)?;
                // Zero while no frame has approved payment, which leaves the
                // transaction invalid regardless.
                vm.frame_tx_context
                    .as_ref()
                    .and_then(|ctx| ctx.payer_address)
                    .map_or(U256::zero(), address_to_u256)
            }
            _ => return Err(ExceptionalHalt::InvalidOpcode.into()),
        };
        vm.current_call_frame.stack.push(value)?;
        Ok(OpcodeResult::Continue)
    }
}

/// `EVENTDATACOPY` (0xB7): copies part of one event's non-indexed data into memory,
/// priced like `CALLDATACOPY`. Unlike `CALLDATACOPY`, a range past the end of the
/// data halts instead of zero-padding, as does an event index past the event count.
pub struct OpEventDataCopyHandler;
impl OpcodeHandler for OpEventDataCopyHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [event_index, mem_offset, data_offset, size] = *vm.current_call_frame.stack.pop()?;
        let (size, mem_offset) = size_offset_to_usize(size, mem_offset)?;
        let new_memory_size = calculate_memory_size(mem_offset, size)?;
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::eventdatacopy(
                new_memory_size,
                vm.current_call_frame.memory.len(),
                size,
            )?)?;
        require_assert_frame(vm)?;

        let event = table_entry(&transaction_events(vm), event_index)?;
        let start = usize::try_from(data_offset).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let end = start
            .checked_add(size)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;
        let data = event
            .data
            .get(start..end)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;
        if size == 0 {
            return Ok(OpcodeResult::Continue);
        }
        vm.current_call_frame.memory.store_data(mem_offset, data)?;
        Ok(OpcodeResult::Continue)
    }
}
