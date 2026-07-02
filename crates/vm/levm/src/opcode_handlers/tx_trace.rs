//! EIP-7906: TXTRACE, EVENTDATACOPY, and TXDIFF data-extraction opcodes.
//! On hegota-devnet these are at 0xB6 / 0xB7 / 0xB8 (renumbered above EIP-8272's
//! RECENTROOTREFLOAD at 0xB5). All three are valid only inside a POST_TX frame.
//!
//! Pure functions that derive the transaction-scoped trace views (balance
//! changes, storage-slot changes, deployed contracts, event topics, and the
//! gas pre-charge) from borrowed state. They take immutable references and
//! return owned results so they can be called from the opcode handlers without
//! a `&mut VM` borrow. The handlers live in this module.

use ethrex_common::constants::EMPTY_KECCAK_HASH;
use ethrex_common::types::{Code, FrameMode, Log};
use ethrex_common::{Address, H256, U256};
use rustc_hash::FxHashMap;

use crate::db::gen_db::CacheDB;
use crate::errors::{ExceptionalHalt, OpcodeResult, VMError};
use crate::gas_cost;
use crate::memory::calculate_memory_size;
use crate::opcode_handlers::OpcodeHandler;
use crate::opcode_handlers::frame_tx::{
    address_to_u256, compute_tx_max_cost, index_to_usize, u256_to_offset,
};
use crate::utils::{
    code_has_delegation, size_offset_to_usize, word_to_address,
};
use crate::vm::VM;

/// Balance changes for the transaction, as `(address, balance_before, balance_after)`.
///
/// Includes every address in `current` whose live balance differs from its
/// prestate balance in `initial` (an address absent from `initial` has a
/// `balance_before` of zero). Sorted by address ascending (uint160 big-endian
/// order, which is `Address`'s natural `Ord`).
pub(crate) fn balance_changes(initial: &CacheDB, current: &CacheDB) -> Vec<(Address, U256, U256)> {
    let mut changes: Vec<(Address, U256, U256)> = current
        .iter()
        .filter_map(|(address, account)| {
            let after = account.info.balance;
            let before = initial
                .get(address)
                .map(|acc| acc.info.balance)
                .unwrap_or(U256::zero());
            (after != before).then_some((*address, before, after))
        })
        .collect();
    changes.sort_by(|a, b| a.0.cmp(&b.0));
    changes
}

/// Storage-slot changes for the transaction, as
/// `(address, slot_key, value_before, value_after)`.
///
/// Includes every `(address, slot)` in `current` storage whose live value
/// differs from its prestate value in `initial` (an absent initial slot has a
/// `value_before` of zero). Sorted by address ascending, then by slot key as a
/// uint256 ascending.
pub(crate) fn slot_changes(
    initial: &CacheDB,
    current: &CacheDB,
) -> Vec<(Address, H256, U256, U256)> {
    let mut changes: Vec<(Address, H256, U256, U256)> = Vec::new();
    for (address, account) in current.iter() {
        let initial_account = initial.get(address);
        for (slot, after) in account.storage.iter() {
            let before = initial_account
                .and_then(|acc| acc.storage.get(slot).copied())
                .unwrap_or(U256::zero());
            if *after != before {
                changes.push((*address, *slot, before, *after));
            }
        }
    }
    changes.sort_by(|a, b| {
        a.0.cmp(&b.0).then_with(|| {
            U256::from_big_endian(a.1.as_bytes()).cmp(&U256::from_big_endian(b.1.as_bytes()))
        })
    });
    changes
}

/// Contracts deployed during the transaction, as `(address, codehash_after)`.
///
/// Includes every address whose prestate code is empty (empty Keccak hash, or
/// the address is absent from `initial`) and whose live code is non-empty,
/// EXCLUDING EIP-7702 delegation designators (`0xef0100 || addr`). Current code
/// bytes are fetched from `codes` by their code hash for the delegation check.
/// Sorted by address ascending. Propagates `VMError` from `code_has_delegation`.
pub(crate) fn deployed_contracts(
    codes: &FxHashMap<H256, Code>,
    initial: &CacheDB,
    current: &CacheDB,
) -> Result<Vec<(Address, H256)>, VMError> {
    let mut deployed: Vec<(Address, H256)> = Vec::new();
    for (address, account) in current.iter() {
        let code_hash_after = account.info.code_hash;
        if code_hash_after == *EMPTY_KECCAK_HASH {
            continue;
        }
        let was_empty = initial
            .get(address)
            .map(|acc| acc.info.code_hash == *EMPTY_KECCAK_HASH)
            .unwrap_or(true);
        if !was_empty {
            continue;
        }
        if let Some(code) = codes.get(&code_hash_after)
            && code_has_delegation(code.code())?
        {
            continue;
        }
        deployed.push((*address, code_hash_after));
    }
    deployed.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(deployed)
}

/// Returns the `n`-th topic of `log`, or `None` if the log has fewer topics.
pub(crate) fn topic_at(log: &Log, n: usize) -> Option<H256> {
    log.topics.get(n).copied()
}

/// Compute the transaction gas pre-charge for a NORMAL transaction:
/// `total_gas_limit * effective_gas_price + blob_count * BLOB_GAS_PER_BLOB * base_blob_fee`.
///
/// Uses checked arithmetic; any overflow returns `ExceptionalHalt::OutOfGas`
/// (matching the overflow convention of the `gas_cost` helpers). The frame-tx
/// path instead reuses `compute_tx_max_cost` — APPROVE debits the maximum
/// cost, so that is a frame transaction's pre-charge.
pub(crate) fn gas_pre_charge(
    total_gas_limit: u64,
    effective_gas_price: U256,
    blob_count: u64,
    base_blob_fee: U256,
) -> Result<U256, VMError> {
    let gas_cost = U256::from(total_gas_limit)
        .checked_mul(effective_gas_price)
        .ok_or(ExceptionalHalt::OutOfGas)?;
    let blob_gas = U256::from(blob_count)
        .checked_mul(U256::from(gas_cost::BLOB_GAS_PER_BLOB))
        .ok_or(ExceptionalHalt::OutOfGas)?;
    let blob_cost = blob_gas
        .checked_mul(base_blob_fee)
        .ok_or(ExceptionalHalt::OutOfGas)?;
    gas_cost
        .checked_add(blob_cost)
        .ok_or(ExceptionalHalt::OutOfGas.into())
}

/// The transaction's logs in global emission order.
///
/// `commit_backup` folds each completed frame's logs into the substate log
/// chain, so `extract_logs` (which walks parent -> child appending each
/// scope's logs) already yields the correct whole-transaction emission order.
/// Do NOT additionally fold in `frame_results` logs: they are the same logs and
/// would be double-counted.
///
/// Returns owned (cloned) logs by design. TXTRACE / EVENTDATACOPY recompute the
/// view on each call, and owning the logs decouples the read from the later
/// mutable borrows of `memory` / `stack` in the handlers.
fn ordered_tx_logs(vm: &VM<'_>) -> Vec<Log> {
    vm.substate.extract_logs()
}

/// TXTRACE (0xB6) -- EIP-7906 transaction-scoped state/event introspection.
///
/// Stack: `[in2, param]` with `in2` on top (popped first) and `param` the
/// deeper operand, matching FRAMEPARAM. `param` selects the field; `in2` is
/// either an index into the relevant list or must be zero for scalar fields.
/// Gas cost: `TXTRACE` (100).
///
/// EIP-7906 (spec PR #11829): TXTRACE / EVENTDATACOPY / TXDIFF may execute ONLY
/// inside a POST_TX frame's call subtree. In any other context — legacy/EIP-1559
/// transactions, or any other EIP-8141 frame mode — they exceptional-halt.
/// `current_frame_index` tracks the enclosing tx frame, so this holds for nested
/// calls within the POST_TX frame's subtree as well.
fn require_post_tx_frame(vm: &VM<'_>) -> Result<(), VMError> {
    let ctx = vm
        .frame_tx_context
        .as_ref()
        .ok_or(ExceptionalHalt::InvalidOpcode)?;
    match ctx
        .tx
        .frames
        .get(ctx.current_frame_index)
        .map(|f| f.execution_mode())
    {
        Some(FrameMode::PostTx) => Ok(()),
        _ => Err(ExceptionalHalt::InvalidOpcode.into()),
    }
}

pub struct OpTxTraceHandler;
impl OpcodeHandler for OpTxTraceHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        require_post_tx_frame(vm)?;
        let [in2, param] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TXTRACE)?;

        let param = u64::try_from(param).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let in2 = u64::try_from(in2).map_err(|_| ExceptionalHalt::InvalidOpcode)?;

        // Compute the owned result first while borrowing VM state immutably;
        // the borrow ends before the stack push below.
        let result: U256 = {
            let initial = &vm.db.initial_accounts_state;
            let current = &vm.db.current_accounts_state;
            match param {
                // -- counts (in2 must be 0) --
                0x00 => {
                    require_zero(in2)?;
                    U256::from(balance_changes(initial, current).len())
                }
                0x01 => {
                    require_zero(in2)?;
                    U256::from(slot_changes(initial, current).len())
                }
                0x02 => {
                    require_zero(in2)?;
                    U256::from(deployed_contracts(&vm.db.codes, initial, current)?.len())
                }
                // -- balance changes (in2 = index) --
                0x03..=0x05 => {
                    let changes = balance_changes(initial, current);
                    let idx = index_to_usize(in2)?;
                    let (address, before, after) =
                        *changes.get(idx).ok_or(ExceptionalHalt::InvalidOpcode)?;
                    match param {
                        0x03 => address_to_u256(address),
                        0x04 => before,
                        _ => after,
                    }
                }
                // -- storage-slot changes (in2 = index) --
                0x06..=0x09 => {
                    let changes = slot_changes(initial, current);
                    let idx = index_to_usize(in2)?;
                    let (address, slot, before, after) =
                        *changes.get(idx).ok_or(ExceptionalHalt::InvalidOpcode)?;
                    match param {
                        0x06 => address_to_u256(address),
                        0x07 => U256::from_big_endian(slot.as_bytes()),
                        0x08 => before,
                        _ => after,
                    }
                }
                // -- deployed contracts (in2 = index) --
                0x0A | 0x0B => {
                    let deployed = deployed_contracts(&vm.db.codes, initial, current)?;
                    let idx = index_to_usize(in2)?;
                    let (address, code_hash) =
                        *deployed.get(idx).ok_or(ExceptionalHalt::InvalidOpcode)?;
                    if param == 0x0A {
                        address_to_u256(address)
                    } else {
                        U256::from_big_endian(code_hash.as_bytes())
                    }
                }
                // -- events count (in2 must be 0) --
                0x0C => {
                    require_zero(in2)?;
                    U256::from(ordered_tx_logs(vm).len())
                }
                // -- event fields (in2 = event index) --
                0x0D..=0x13 => {
                    let logs = ordered_tx_logs(vm);
                    let idx = index_to_usize(in2)?;
                    let log = logs.get(idx).ok_or(ExceptionalHalt::InvalidOpcode)?;
                    match param {
                        0x0D => address_to_u256(log.address),
                        0x0E => U256::from(log.topics.len()),
                        // 0x0F..=0x12 -> topic0..topic3; halt if the topic is absent.
                        0x0F..=0x12 => {
                            // Map the param literal to its topic index directly so
                            // there is no subtraction to overflow-check.
                            let n = match param {
                                0x0F => 0,
                                0x10 => 1,
                                0x11 => 2,
                                _ => 3,
                            };
                            let topic = topic_at(log, n).ok_or(ExceptionalHalt::InvalidOpcode)?;
                            U256::from_big_endian(topic.as_bytes())
                        }
                        _ => U256::from(log.data.len()),
                    }
                }
                // -- gas pre-charge (in2 must be 0) --
                0x14 => {
                    require_zero(in2)?;
                    if let Some(ctx) = vm.frame_tx_context.as_ref() {
                        // Frame tx: APPROVE debits the maximum cost, so that is
                        // the pre-charge this param reports.
                        compute_tx_max_cost(ctx)?
                    } else {
                        gas_pre_charge(
                            vm.env.gas_limit,
                            vm.env.gas_price,
                            u64::try_from(vm.env.tx_blob_hashes.len())
                                .map_err(|_| ExceptionalHalt::InvalidOpcode)?,
                            vm.env.base_blob_fee_per_gas,
                        )?
                    }
                }
                // -- gas payer (in2 must be 0) --
                0x15 => {
                    require_zero(in2)?;
                    let payer = vm
                        .frame_tx_context
                        .as_ref()
                        .and_then(|c| c.payer_address)
                        .unwrap_or(vm.env.origin);
                    address_to_u256(payer)
                }
                _ => return Err(ExceptionalHalt::InvalidOpcode.into()),
            }
        };

        vm.current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Reject a non-zero `in2` operand on a scalar (must-be-0) TXTRACE param.
fn require_zero(in2: u64) -> Result<(), VMError> {
    if in2 != 0 {
        return Err(ExceptionalHalt::InvalidOpcode.into());
    }
    Ok(())
}

/// EVENTDATACOPY (0xB7) -- EIP-7906 copy of an emitted event's data into memory.
///
/// Mirrors CALLDATACOPY's gas accounting, but past-the-end reads halt (the data
/// region is exactly `data[data_offset..data_offset+length]`; no zero-fill).
/// Gas cost matches CALLDATACOPY.
pub struct OpEventDataCopyHandler;
impl OpcodeHandler for OpEventDataCopyHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        require_post_tx_frame(vm)?;
        // EIP-7906 stack: event_index(top), memOffset, dataOffset, length
        // NOTE: differs from FRAMEDATACOPY which has the index at the bottom.
        let [event_index, mem_offset, data_offset, length] = *vm.current_call_frame.stack.pop()?;
        let (length, mem_offset) = size_offset_to_usize(length, mem_offset)?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::calldatacopy(
                calculate_memory_size(mem_offset, length)?,
                vm.current_call_frame.memory.len(),
                length,
            )?)?;

        let event_index = u64::try_from(event_index).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let event_index = index_to_usize(event_index)?;
        // Past-the-end data offsets are a halt (no zero-fill), so the offset
        // must resolve to a real usize.
        let data_offset = u256_to_offset(data_offset).ok_or(ExceptionalHalt::InvalidOpcode)?;

        // `logs` is owned (cloned) so slicing it does not conflict with the
        // `&mut memory` borrow in the store below.
        let logs = ordered_tx_logs(vm);
        // event_index is validated even when length == 0; `.get` keeps this
        // panic-proof in addition to the explicit bounds semantics.
        let log = logs
            .get(event_index)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;
        let data = &log.data;
        let end = data_offset
            .checked_add(length)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;
        if end > data.len() {
            return Err(ExceptionalHalt::InvalidOpcode.into());
        }

        if length == 0 {
            return Ok(OpcodeResult::Continue);
        }

        // `data_offset..end` was bounds-checked above (`end <= data.len()`); use
        // `.get` so the slice is panic-proof regardless.
        let chunk = data
            .get(data_offset..end)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;
        vm.current_call_frame.memory.store_data(mem_offset, chunk)?;

        Ok(OpcodeResult::Continue)
    }
}

/// TXDIFF (0xB8) -- EIP-7906 keyed state-diff lookup (spec PR #11830).
///
/// Stack: `[param, address, in3]` with `param` on top (popped first), then
/// `address`, then `in3` (deepest). `param` selects the field; `address` is the
/// account (low 20 bytes of the word); `in3` is the storage-slot key for the
/// slot params and MUST be zero for the scalar (balance / codehash) params.
///
/// Params: `0x00` slot_before / `0x01` slot_after / `0x02` balance_before /
/// `0x03` balance_after / `0x04` codehash_before / `0x05` codehash_after.
///
/// "before" is the transaction prestate (the value held in
/// `initial_accounts_state`); "after" is the live post-body value (in
/// `current_accounts_state`, which inside a POST_TX frame already reflects the
/// whole executed tx body). A key the transaction never modified yields the same
/// live value for both directions; an undeployed account's codehash_before is
/// the empty-Keccak hash. The reads load the account/slot into the diff caches
/// if absent but never trigger EIP-2929 warm/cold accounting — TXDIFF has a flat
/// gas cost. Valid only inside a POST_TX frame (like TXTRACE / EVENTDATACOPY).
pub struct OpTxDiffHandler;
impl OpcodeHandler for OpTxDiffHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        require_post_tx_frame(vm)?;
        let [param, address, in3] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TXDIFF)?;

        let param = u64::try_from(param).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let address = word_to_address(address);

        let result: U256 = match param {
            // -- storage slot (in3 = slot key) --
            0x00 | 0x01 => {
                let key = H256(in3.to_big_endian());
                // `get_storage_value` returns the live (post-body) value and, on a
                // read-path miss, caches it into both `current` and `initial`; it
                // errors if the account is not yet loaded, so load it first.
                vm.db
                    .get_account(address)
                    .map_err(|_| ExceptionalHalt::InvalidOpcode)?;
                let after = vm
                    .get_storage_value(address, key)
                    .map_err(|_| ExceptionalHalt::InvalidOpcode)?;
                if param == 0x01 {
                    after
                } else {
                    // slot_before: the prestate value lives in `initial`. The read
                    // above guarantees the slot is present there (every key in
                    // `current.storage` is also in `initial.storage`); fall back to
                    // the live value so an unmodified slot still reads before==after.
                    vm.db
                        .initial_accounts_state
                        .get(&address)
                        .and_then(|acc| acc.storage.get(&key).copied())
                        .unwrap_or(after)
                }
            }
            // -- balance (in3 must be 0) --
            0x02 | 0x03 => {
                require_zero_word(in3)?;
                vm.db
                    .get_account(address)
                    .map_err(|_| ExceptionalHalt::InvalidOpcode)?;
                let after = vm
                    .db
                    .current_accounts_state
                    .get(&address)
                    .map(|acc| acc.info.balance)
                    .unwrap_or(U256::zero());
                if param == 0x03 {
                    after
                } else {
                    vm.db
                        .initial_accounts_state
                        .get(&address)
                        .map(|acc| acc.info.balance)
                        .unwrap_or(after)
                }
            }
            // -- code hash (in3 must be 0) --
            0x04 | 0x05 => {
                require_zero_word(in3)?;
                vm.db
                    .get_account(address)
                    .map_err(|_| ExceptionalHalt::InvalidOpcode)?;
                let after = vm
                    .db
                    .current_accounts_state
                    .get(&address)
                    .map(|acc| acc.info.code_hash)
                    .unwrap_or(*EMPTY_KECCAK_HASH);
                let hash = if param == 0x05 {
                    after
                } else {
                    vm.db
                        .initial_accounts_state
                        .get(&address)
                        .map(|acc| acc.info.code_hash)
                        .unwrap_or(after)
                };
                U256::from_big_endian(hash.as_bytes())
            }
            _ => return Err(ExceptionalHalt::InvalidOpcode.into()),
        };

        vm.current_call_frame.stack.push(result)?;
        Ok(OpcodeResult::Continue)
    }
}

/// Reject a non-zero `in3` operand on a scalar (must-be-0) TXDIFF param.
fn require_zero_word(in3: U256) -> Result<(), VMError> {
    if !in3.is_zero() {
        return Err(ExceptionalHalt::InvalidOpcode.into());
    }
    Ok(())
}

#[cfg(test)]
mod pure_fn_tests {
    //! Unit tests for the transaction-scoped trace views (the pure functions that
    //! TXTRACE / TXDIFF read). These exercise the diff computation directly from
    //! hand-built prestate (`initial`) and live (`current`) caches, independent of
    //! the opcode dispatch and frame machinery (covered by the integration tests
    //! in `test/tests/levm/eip7906_tests.rs`).

    use super::*;
    use crate::account::{AccountStatus, LevmAccount};
    use ethrex_common::types::AccountInfo;

    fn addr(n: u64) -> Address {
        Address::from_low_u64_be(n)
    }

    fn slot(n: u64) -> H256 {
        H256::from_low_u64_be(n)
    }

    fn slot_num(s: &H256) -> u64 {
        U256::from_big_endian(s.as_bytes()).low_u64()
    }

    /// A LevmAccount with `balance`, `code_hash`, and `(slot, value)` storage.
    fn acct(balance: u64, code_hash: H256, slots: &[(u64, u64)]) -> LevmAccount {
        let storage = slots
            .iter()
            .map(|(k, v)| (slot(*k), U256::from(*v)))
            .collect();
        LevmAccount {
            info: AccountInfo {
                code_hash,
                balance: U256::from(balance),
                nonce: 0,
            },
            storage,
            has_storage: !slots.is_empty(),
            status: AccountStatus::Modified,
            exists: true,
        }
    }

    fn empty_hash() -> H256 {
        *EMPTY_KECCAK_HASH
    }

    fn cache(entries: Vec<(Address, LevmAccount)>) -> CacheDB {
        entries.into_iter().collect()
    }

    fn code_of(bytes: Vec<u8>) -> Code {
        Code::from_bytecode(bytes::Bytes::from(bytes), &ethrex_crypto::NativeCrypto)
    }

    // ---------------- balance_changes ----------------

    #[test]
    fn balance_changes_excludes_net_zero_and_reports_before_after() {
        let initial = cache(vec![
            (addr(1), acct(100, empty_hash(), &[])),
            (addr(2), acct(50, empty_hash(), &[])),
        ]);
        let current = cache(vec![
            (addr(1), acct(150, empty_hash(), &[])), // +50 -> included
            (addr(2), acct(50, empty_hash(), &[])),  // net-zero -> excluded
        ]);
        assert_eq!(
            balance_changes(&initial, &current),
            vec![(addr(1), U256::from(100), U256::from(150))]
        );
    }

    #[test]
    fn balance_before_is_zero_when_absent_from_prestate() {
        let initial = cache(vec![]);
        let current = cache(vec![(addr(7), acct(42, empty_hash(), &[]))]);
        assert_eq!(
            balance_changes(&initial, &current),
            vec![(addr(7), U256::zero(), U256::from(42))]
        );
    }

    #[test]
    fn balance_changes_sorted_by_address() {
        let initial = cache(vec![]);
        let current = cache(vec![
            (addr(3), acct(3, empty_hash(), &[])),
            (addr(1), acct(1, empty_hash(), &[])),
            (addr(2), acct(2, empty_hash(), &[])),
        ]);
        let got: Vec<Address> = balance_changes(&initial, &current)
            .iter()
            .map(|(a, ..)| *a)
            .collect();
        assert_eq!(got, vec![addr(1), addr(2), addr(3)]);
    }

    // ---------------- slot_changes ----------------

    #[test]
    fn slot_changes_excludes_restored_slot_and_reports_before_after() {
        let initial = cache(vec![(addr(1), acct(0, empty_hash(), &[(0, 10), (1, 20)]))]);
        // slot 0 restored to its original 10 (excluded); slot 1 changed 20 -> 99.
        let current = cache(vec![(addr(1), acct(0, empty_hash(), &[(0, 10), (1, 99)]))]);
        assert_eq!(
            slot_changes(&initial, &current),
            vec![(addr(1), slot(1), U256::from(20), U256::from(99))]
        );
    }

    #[test]
    fn slot_before_is_zero_when_absent_from_prestate() {
        let initial = cache(vec![(addr(1), acct(0, empty_hash(), &[]))]);
        let current = cache(vec![(addr(1), acct(0, empty_hash(), &[(5, 7)]))]);
        assert_eq!(
            slot_changes(&initial, &current),
            vec![(addr(1), slot(5), U256::zero(), U256::from(7))]
        );
    }

    #[test]
    fn slot_changes_sorted_by_address_then_slot() {
        let initial = cache(vec![]);
        let current = cache(vec![
            (addr(2), acct(0, empty_hash(), &[(1, 1)])),
            (addr(1), acct(0, empty_hash(), &[(2, 1), (1, 1)])),
        ]);
        let got: Vec<(Address, u64)> = slot_changes(&initial, &current)
            .iter()
            .map(|(a, s, ..)| (*a, slot_num(s)))
            .collect();
        assert_eq!(got, vec![(addr(1), 1), (addr(1), 2), (addr(2), 1)]);
    }

    // ---------------- deployed_contracts ----------------

    #[test]
    fn deployed_contracts_counts_new_code_excludes_preexisting() {
        let new_code = code_of(vec![0x60, 0x00]);
        let pre_code = code_of(vec![0x60, 0x01]);
        let mut codes = FxHashMap::default();
        codes.insert(new_code.hash, new_code.clone());
        codes.insert(pre_code.hash, pre_code.clone());
        let initial = cache(vec![
            (addr(1), acct(0, empty_hash(), &[])),  // undeployed
            (addr(2), acct(0, pre_code.hash, &[])), // already had code
        ]);
        let current = cache(vec![
            (addr(1), acct(0, new_code.hash, &[])), // deployed this tx
            (addr(2), acct(0, pre_code.hash, &[])),
        ]);
        assert_eq!(
            deployed_contracts(&codes, &initial, &current).unwrap(),
            vec![(addr(1), new_code.hash)]
        );
    }

    #[test]
    fn deployed_contracts_excludes_7702_delegation_designator() {
        // EIP-7702 designator: 0xef0100 || 20-byte address (23 bytes).
        let mut designator = vec![0xef, 0x01, 0x00];
        designator.extend_from_slice(addr(0xDE).as_bytes());
        let deleg = code_of(designator);
        let mut codes = FxHashMap::default();
        codes.insert(deleg.hash, deleg.clone());
        let initial = cache(vec![(addr(1), acct(0, empty_hash(), &[]))]);
        let current = cache(vec![(addr(1), acct(0, deleg.hash, &[]))]);
        assert!(
            deployed_contracts(&codes, &initial, &current)
                .unwrap()
                .is_empty(),
            "an EIP-7702 delegation must not count as a contract deployment"
        );
    }

    #[test]
    fn deployed_contracts_sorted_by_address() {
        let c = code_of(vec![0x60, 0x00]);
        let mut codes = FxHashMap::default();
        codes.insert(c.hash, c.clone());
        let initial = cache(vec![]);
        let current = cache(vec![
            (addr(3), acct(0, c.hash, &[])),
            (addr(1), acct(0, c.hash, &[])),
            (addr(2), acct(0, c.hash, &[])),
        ]);
        let got: Vec<Address> = deployed_contracts(&codes, &initial, &current)
            .unwrap()
            .iter()
            .map(|(a, _)| *a)
            .collect();
        assert_eq!(got, vec![addr(1), addr(2), addr(3)]);
    }
}
