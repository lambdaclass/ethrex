//! EIP-7906 transaction assertions: `TXTRACE` (0xB6), `TXDIFF` (0xB7) and
//! `EVENTDATACOPY` (0xB8).
//!
//! The three opcodes expose the executing frame transaction's state diff to its
//! `POST_TX` frames, and halt exceptionally anywhere else. The diff is the net
//! difference between the transaction prestate (`GeneralizedDatabase::tx_prestate`,
//! each account and slot as the transaction first touched it) and the live state at
//! the reading instruction. The tables are recomputed on every read from those two
//! maps and from the logs of the frames that have completed.

use ethrex_common::constants::EMPTY_KECCAK_HASH;
use ethrex_common::types::{FrameMode, Log};
use ethrex_common::{Address, H256, U256};

use crate::db::gen_db::CacheDB;
use crate::errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError};
use crate::gas_cost;
use crate::memory::calculate_memory_size;
use crate::opcode_handlers::OpcodeHandler;
use crate::opcode_handlers::frame_tx::{address_to_u256, compute_tx_max_cost};
use crate::utils::{code_has_delegation, size_offset_to_usize, word_to_address};
use crate::vm::VM;

/// `account_change_flags` bits, in the field order of the account tuple
/// `(nonce, balance, storage_root, code_hash)`.
const CHANGE_FLAG_NONCE: u8 = 0b0001;
const CHANGE_FLAG_BALANCE: u8 = 0b0010;
const CHANGE_FLAG_STORAGE: u8 = 0b0100;
const CHANGE_FLAG_CODE: u8 = 0b1000;

/// One `balances_changed` entry: `(address, balance_before, balance_after)`.
type BalanceChange = (Address, U256, U256);
/// One `slots_changed` entry: `(address, slot_key, value_before, value_after)`.
type SlotChange = (Address, H256, U256, U256);
/// One `contracts_deployed` entry: `(address, codehash_after)`.
type DeployedContract = (Address, H256);

/// The `balances_changed` table: every address the transaction touched whose live
/// balance differs from its prestate balance, ascending by address.
///
/// The touched set is the prestate map, not the live cache: the cache can span
/// several transactions, so iterating it would report other transactions'
/// changes. An account enters the prestate map on the same first touch that puts
/// it in the cache, so a prestate entry always has a live one.
fn balance_changes(prestate: &CacheDB, current: &CacheDB) -> Vec<BalanceChange> {
    let mut changes: Vec<BalanceChange> = prestate
        .iter()
        .filter_map(|(address, account)| {
            let before = account.info.balance;
            let after = current
                .get(address)
                .map_or(before, |live| live.info.balance);
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

/// The `contracts_deployed` table: every address whose code went from empty to
/// code that is not an EIP-7702 delegation designator, ascending by address.
fn deployed_contracts(vm: &VM<'_>, prestate: &CacheDB) -> Result<Vec<DeployedContract>, VMError> {
    let mut deployed: Vec<DeployedContract> = Vec::new();
    for (address, account) in prestate {
        if account.info.code_hash != *EMPTY_KECCAK_HASH {
            continue;
        }
        let after = vm
            .db
            .current_accounts_state
            .get(address)
            .map_or(*EMPTY_KECCAK_HASH, |live| live.info.code_hash);
        if after == *EMPTY_KECCAK_HASH {
            continue;
        }
        if let Some(code) = vm.db.codes.get(&after)
            && code_has_delegation(code.code())?
        {
            continue;
        }
        deployed.push((*address, after));
    }
    deployed.sort_by_key(|entry| entry.0);
    Ok(deployed)
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

/// `account_change_flags` for `address`. An address outside the prestate map was
/// never touched by the transaction, so every bit is clear and no state is read.
fn account_change_flags(
    prestate: &CacheDB,
    current: &CacheDB,
    slots: &[SlotChange],
    address: Address,
) -> U256 {
    let (Some(before), Some(after)) = (prestate.get(&address), current.get(&address)) else {
        return U256::zero();
    };
    let mut flags = 0u8;
    if before.info.nonce != after.info.nonce {
        flags |= CHANGE_FLAG_NONCE;
    }
    if before.info.balance != after.info.balance {
        flags |= CHANGE_FLAG_BALANCE;
    }
    if slots.iter().any(|change| change.0 == address) {
        flags |= CHANGE_FLAG_STORAGE;
    }
    if before.info.code_hash != after.info.code_hash {
        flags |= CHANGE_FLAG_CODE;
    }
    U256::from(flags)
}

/// Halts unless the executing frame is a `POST_TX` frame. The check keys on the
/// frame, not the call depth, so the opcodes work anywhere in a `POST_TX` frame's
/// call subtree and nowhere else.
fn require_post_tx_frame(vm: &VM<'_>) -> Result<(), VMError> {
    let ctx = vm
        .frame_tx_context
        .as_ref()
        .ok_or(ExceptionalHalt::InvalidOpcode)?;
    match ctx.tx.frames.get(ctx.current_frame_index) {
        Some(frame) if frame.execution_mode() == FrameMode::PostTx => Ok(()),
        _ => Err(ExceptionalHalt::InvalidOpcode.into()),
    }
}

/// The transaction prestate. `VM::new` installs it for every transaction carrying a
/// `POST_TX` frame, and `require_post_tx_frame` has established that this is one, so
/// its absence is a broken invariant rather than something the opcodes can observe.
fn tx_prestate<'a>(vm: &'a VM<'_>) -> Result<&'a CacheDB, VMError> {
    vm.db
        .tx_prestate
        .as_ref()
        .ok_or_else(|| InternalError::msg("EIP-7906 opcode without a transaction prestate").into())
}

/// Halts unless an operand the parameter tables mark *must be 0* is zero.
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

fn u256_to_h256(value: U256) -> H256 {
    H256(value.to_big_endian())
}

fn h256_to_u256(value: H256) -> U256 {
    U256::from_big_endian(value.as_bytes())
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
        require_post_tx_frame(vm)?;

        let prestate = tx_prestate(vm)?;
        let current = &vm.db.current_accounts_state;
        let param = u8::try_from(param).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let value = match param {
            0x00 => {
                require_zero(index)?;
                U256::from(balance_changes(prestate, current).len())
            }
            0x01 => {
                require_zero(index)?;
                U256::from(slot_changes(prestate, current).len())
            }
            0x02 => {
                require_zero(index)?;
                U256::from(deployed_contracts(vm, prestate)?.len())
            }
            0x03..=0x05 => {
                let (address, before, after) =
                    table_entry(&balance_changes(prestate, current), index)?;
                match param {
                    0x03 => address_to_u256(address),
                    0x04 => before,
                    _ => after,
                }
            }
            0x06..=0x09 => {
                let (address, key, before, after) =
                    table_entry(&slot_changes(prestate, current), index)?;
                match param {
                    0x06 => address_to_u256(address),
                    0x07 => h256_to_u256(key),
                    0x08 => before,
                    _ => after,
                }
            }
            0x0A | 0x0B => {
                let (address, code_hash) = table_entry(&deployed_contracts(vm, prestate)?, index)?;
                if param == 0x0A {
                    address_to_u256(address)
                } else {
                    h256_to_u256(code_hash)
                }
            }
            0x0C => {
                require_zero(index)?;
                U256::from(transaction_events(vm).len())
            }
            0x0D..=0x13 => {
                let event = table_entry(&transaction_events(vm), index)?;
                match param {
                    0x0D => address_to_u256(event.address),
                    0x0E => U256::from(event.topics.len()),
                    0x0F => event_topic(&event, 0)?,
                    0x10 => event_topic(&event, 1)?,
                    0x11 => event_topic(&event, 2)?,
                    0x12 => event_topic(&event, 3)?,
                    _ => U256::from(event.data.len()),
                }
            }
            0x14 => {
                require_zero(index)?;
                // The payer's escrow: the transaction's maximum cost, blob fees
                // included, collected at payment approval.
                let ctx = vm
                    .frame_tx_context
                    .as_ref()
                    .ok_or(ExceptionalHalt::InvalidOpcode)?;
                compute_tx_max_cost(ctx)?
            }
            0x15 => {
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

/// `TXDIFF` (0xB7): a keyed lookup into the transaction's state diff. Stack:
/// `param` (top), then an address or topic `key`, then a slot key, per-key index or
/// zero.
///
/// Params `0x00` to `0x05` read one slot, balance or code hash before and after the
/// transaction. A key the transaction never touched is read live, so they are
/// priced, warmed and recorded in the block access list like any other state read.
/// Params `0x06` to `0x0C` are answered from the diff alone at the flat `TXTRACE`
/// cost.
pub struct OpTxDiffHandler;
impl OpcodeHandler for OpTxDiffHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [param, key, index] = *vm.current_call_frame.stack.pop()?;
        let address = word_to_address(key);
        let slot = u256_to_h256(index);
        let param = u8::try_from(param).unwrap_or(u8::MAX);

        // Gas, and the EIP-2929 access it implies, before anything else.
        let fork = vm.env.config.fork;
        let cost = match param {
            0x00 | 0x01 => gas_cost::sload(vm.substate.add_accessed_slot(address, slot), fork)?,
            0x02..=0x05 => gas_cost::balance(vm.substate.add_accessed_address(address), fork)?,
            _ => gas_cost::TXTRACE,
        };
        vm.current_call_frame.increase_consumed_gas(cost)?;
        require_post_tx_frame(vm)?;

        let value = match param {
            0x00 | 0x01 => {
                // The live read captures the slot's prestate if this is the
                // transaction's first touch, so an untouched slot reads the same value
                // before and after.
                vm.db.get_account(address)?;
                let live = vm.get_storage_value(address, slot)?;
                vm.record_storage_slot_to_bal(address, index);
                if param == 0x00 {
                    tx_prestate(vm)?
                        .get(&address)
                        .and_then(|account| account.storage.get(&slot).copied())
                        .unwrap_or(live)
                } else {
                    live
                }
            }
            0x02..=0x05 => {
                require_zero(index)?;
                let live = vm.db.get_account(address)?.info.clone();
                if let Some(recorder) = vm.db.bal_recorder.as_mut() {
                    recorder.record_touched_address(address);
                }
                let before = tx_prestate(vm)?
                    .get(&address)
                    .map_or_else(|| live.clone(), |account| account.info.clone());
                match param {
                    0x02 => before.balance,
                    0x03 => live.balance,
                    0x04 => h256_to_u256(before.code_hash),
                    _ => h256_to_u256(live.code_hash),
                }
            }
            0x06..=0x0A => {
                let prestate = tx_prestate(vm)?;
                let current = &vm.db.current_accounts_state;
                match param {
                    0x06 | 0x07 => {
                        let indices: Vec<usize> = slot_changes(prestate, current)
                            .iter()
                            .enumerate()
                            .filter_map(|(i, change)| (change.0 == address).then_some(i))
                            .collect();
                        if param == 0x06 {
                            require_zero(index)?;
                            U256::from(indices.len())
                        } else {
                            U256::from(table_entry(&indices, index)?)
                        }
                    }
                    0x08 | 0x09 => {
                        let indices: Vec<usize> = transaction_events(vm)
                            .iter()
                            .enumerate()
                            .filter_map(|(i, event)| (event.address == address).then_some(i))
                            .collect();
                        if param == 0x08 {
                            require_zero(index)?;
                            U256::from(indices.len())
                        } else {
                            U256::from(table_entry(&indices, index)?)
                        }
                    }
                    _ => {
                        require_zero(index)?;
                        let slots = slot_changes(prestate, current);
                        account_change_flags(prestate, current, &slots, address)
                    }
                }
            }
            0x0B | 0x0C => {
                // Keyed by the full topic value. `topic0`, the event signature, is
                // excluded: it identifies the event type, not a participant.
                let topic = u256_to_h256(key);
                let indices: Vec<usize> = transaction_events(vm)
                    .iter()
                    .enumerate()
                    .filter_map(|(i, event)| {
                        event
                            .topics
                            .iter()
                            .skip(1)
                            .any(|candidate| *candidate == topic)
                            .then_some(i)
                    })
                    .collect();
                if param == 0x0B {
                    require_zero(index)?;
                    U256::from(indices.len())
                } else {
                    U256::from(table_entry(&indices, index)?)
                }
            }
            _ => return Err(ExceptionalHalt::InvalidOpcode.into()),
        };
        vm.current_call_frame.stack.push(value)?;
        Ok(OpcodeResult::Continue)
    }
}

/// `EVENTDATACOPY` (0xB8): copies part of one event's non-indexed data into memory,
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
        require_post_tx_frame(vm)?;

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
