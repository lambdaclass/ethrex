use crate::{
    account::LevmAccount,
    constants::*,
    errors::{ContextResult, ExceptionalHalt, InternalError, TxValidationError, VMError},
    gas_cost::{
        self, STANDARD_TOKEN_COST, floor_tokens_in_access_list, total_cost_floor_per_token,
    },
    hooks::hook::Hook,
    utils::*,
    vm::VM,
};

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{Code, Fork},
};

pub const MAX_REFUND_QUOTIENT: u64 = 5;

pub struct DefaultHook;

impl Hook for DefaultHook {
    /// ## Description
    /// This method performs validations and returns an error if any of these fail.
    /// It also makes pre-execution changes:
    /// - It increases sender nonce
    /// - It substracts up-front-cost from sender balance.
    /// - It adds value to receiver balance.
    /// - It calculates and adds intrinsic gas to the 'gas used' of callframe and environment.
    ///   See 'docs' for more information about validations.
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), VMError> {
        let sender_address = vm.env.origin;
        let sender_info = vm.db.get_account(sender_address)?.info.clone();

        if vm.env.config.fork >= Fork::Prague {
            validate_min_gas_limit(vm)?;
            // EIP-7825 (Osaka to pre-Amsterdam): reject tx if gas_limit > POST_OSAKA_GAS_LIMIT_CAP.
            // Amsterdam removes this restriction (EIP-8037 reservoir model).
            if vm.env.config.fork >= Fork::Osaka
                && vm.env.config.fork < Fork::Amsterdam
                && vm.tx.gas_limit() > POST_OSAKA_GAS_LIMIT_CAP
            {
                return Err(VMError::TxValidation(
                    TxValidationError::TxMaxGasLimitExceeded {
                        tx_hash: vm.tx.hash(),
                        tx_gas_limit: vm.tx.gas_limit(),
                    },
                ));
            }
        }

        // (1) GASLIMIT_PRICE_PRODUCT_OVERFLOW
        let gaslimit_price_product = vm
            .env
            .gas_price
            .checked_mul(vm.env.gas_limit.into())
            .ok_or(TxValidationError::GasLimitPriceProductOverflow)?;

        validate_sender_balance(vm, sender_info.balance)?;

        // (2) INSUFFICIENT_MAX_FEE_PER_BLOB_GAS
        if let Some(tx_max_fee_per_blob_gas) = vm.env.tx_max_fee_per_blob_gas {
            validate_max_fee_per_blob_gas(vm, tx_max_fee_per_blob_gas)?;
        }

        // (3) INSUFFICIENT_ACCOUNT_FUNDS
        deduct_caller(vm, gaslimit_price_product, sender_address)?;

        // (4) INSUFFICIENT_MAX_FEE_PER_GAS
        validate_sufficient_max_fee_per_gas(vm)?;

        // (5) INITCODE_SIZE_EXCEEDED
        if vm.is_create()? {
            validate_init_code_size(vm)?;
        }

        // (6) INTRINSIC_GAS_TOO_LOW
        vm.add_intrinsic_gas()?;

        // (7) NONCE_IS_MAX
        vm.increment_account_nonce(sender_address)
            .map_err(|_| TxValidationError::NonceIsMax)?;

        // check for nonce mismatch
        if sender_info.nonce != vm.env.tx_nonce {
            return Err(TxValidationError::NonceMismatch {
                expected: sender_info.nonce,
                actual: vm.env.tx_nonce,
            }
            .into());
        }

        // (8) PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS
        if let (Some(tx_max_priority_fee), Some(tx_max_fee_per_gas)) = (
            vm.env.tx_max_priority_fee_per_gas,
            vm.env.tx_max_fee_per_gas,
        ) && tx_max_priority_fee > tx_max_fee_per_gas
        {
            return Err(TxValidationError::PriorityGreaterThanMaxFeePerGas {
                priority_fee: tx_max_priority_fee,
                max_fee_per_gas: tx_max_fee_per_gas,
            }
            .into());
        }

        // (9) SENDER_NOT_EOA
        let code = vm.db.get_code(sender_info.code_hash)?;
        validate_sender(sender_address, &code.bytecode)?;

        // (10) GAS_ALLOWANCE_EXCEEDED
        validate_gas_allowance(vm)?;

        // Transaction is type 3 if tx_max_fee_per_blob_gas is Some
        if vm.env.tx_max_fee_per_blob_gas.is_some() {
            validate_4844_tx(vm)?;
        }

        // [EIP-7702]: https://eips.ethereum.org/EIPS/eip-7702
        // Transaction is type 4 if authorization_list is Some
        if vm.tx.authorization_list().is_some() {
            validate_type_4_tx(vm)?;
        }

        transfer_value(vm)?;

        set_bytecode_and_code_address(vm)?;

        Ok(())
    }

    /// ## Changes post execution
    /// 1. Undo value transfer if the transaction was reverted
    /// 2. Return unused gas + gas refunds to the sender.
    /// 3. Pay coinbase fee
    /// 4. Destruct addresses in selfdestruct set.
    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        ctx_result: &mut ContextResult,
    ) -> Result<(), VMError> {
        if !ctx_result.is_success() {
            undo_value_transfer(vm)?;
        }

        settle_state_diff_finalized(vm, ctx_result.is_success())?;

        // EIP-8037 (Amsterdam+): Handle CREATE collision specially.
        // Per EELS, collision at process_message_call level returns
        // gas_left=0, state_gas_left=0, regular_gas_used=0, state_gas_used=0.
        // The user pays tx.gas (everything), but block accounting only sees
        // intrinsic gas (no execution gas was consumed).
        if vm.env.config.fork >= Fork::Amsterdam && ctx_result.is_collision() {
            let gas_limit = vm.env.gas_limit;
            // EIP-8037: state_gas derived from finalized StateDiff (intrinsic seed only on collision).
            let state_gas = vm
                .state_diff_finalized
                .bytes()
                .saturating_mul(vm.cost_per_state_byte);
            let floor = vm.get_min_gas_used()?;
            // Regular gas from intrinsic only (gas_limit - reservoir - gas_remaining at collision)
            // = total_intrinsic_gas consumed so far, minus state portion
            #[expect(
                clippy::as_conversions,
                reason = "gas_remaining is positive at collision"
            )]
            let gas_remaining = vm.current_call_frame.gas_remaining as u64;
            let total_intrinsic = gas_limit
                .saturating_sub(vm.state_gas_reservoir)
                .saturating_sub(gas_remaining);
            let regular_gas = total_intrinsic.saturating_sub(state_gas);
            let effective_regular = regular_gas.max(floor);
            ctx_result.gas_used = effective_regular
                .checked_add(state_gas)
                .ok_or(InternalError::Overflow)?;
            // User pays everything (gas_left=0, state_gas_left=0)
            ctx_result.gas_spent = gas_limit;
            // Coinbase gets paid on what user pays
            pay_coinbase(vm, gas_limit)?;
            // Return 0 gas to sender (they lose everything)
            return Ok(());
        }

        // EIP-8037 state-diff: the legacy `apply_same_tx_selfdestruct_state_refund` helper is
        // gone — same-tx CREATE+SELFDESTRUCT cancellation is now handled by the sweep above
        // that calls `state_diff_finalized.cancel_new_account(addr)` for each created+destroyed
        // address in `vm.substate.iter_selfdestruct()`.

        // EIP-8037 (Amsterdam+): unused reservoir is always returned to sender.
        // Per EELS, state_gas_left is preserved even on exceptional halt — only
        // regular gas_left is burned.  The user does NOT pay for unspent reservoir.
        // On top-level failure (revert/halt/oog), execution-time state-gas draws
        // (both from reservoir and spilled to gas_remaining) are refunded too,
        // since the corresponding state operations were rolled back. The
        // post-setup snapshot captures the reservoir balance after intrinsic +
        // auth processing; the spill counter tracks gas_remaining bytes that
        // would otherwise be charged to the user for now-rolled-back state ops.
        if vm.env.config.fork >= Fork::Amsterdam {
            let to_subtract = if ctx_result.is_success() {
                vm.state_gas_reservoir
            } else {
                // EIP-8037 v8037.0.1 incorporate_tx_on_error: per EELS,
                // `tx_output.state_gas_reservoir += max(0, state_gas_used);
                //  state_gas_used = 0` then `total_remaining = gas_left + reservoir`
                // and `tx_gas_used = tx.gas - total_remaining`. Equivalently:
                // refund the user for the unspent reservoir PLUS the execution-time
                // state-gas that was successfully propagated to the top frame
                // (state_gas_used). State-gas drawn by sub-frames whose state changes
                // were reverted (incorporate_child_on_error doesn't propagate
                // state_gas_used) stays charged — those draws are reflected in
                // raw_consumed but not in top.state_gas_used.
                #[expect(clippy::as_conversions, reason = "max(0) keeps in u64 range")]
                let state_gas_used_top = vm.current_call_frame.state_gas_used.max(0) as u64;
                let intrinsic_state_gas_seed = vm
                    .state_diff_intrinsic_seed
                    .bytes()
                    .saturating_mul(vm.cost_per_state_byte);
                // Don't refund the intrinsic-state portion (auth tuples, CREATE-tx
                // target charge) — that's already paid by the user as part of intrinsic.
                let execution_state_gas_used = state_gas_used_top
                    .saturating_sub(intrinsic_state_gas_seed);
                vm.state_gas_reservoir.saturating_add(execution_state_gas_used)
            };
            ctx_result.gas_used = ctx_result.gas_used.saturating_sub(to_subtract);
        }

        // Save pre-refund gas for EIP-7778 block accounting
        let gas_used_pre_refund = ctx_result.gas_used;

        // Note: compute_gas_refunded caps at gas_used / MAX_REFUND_QUOTIENT, where
        // gas_used already has the reservoir subtracted (line above). This matches
        // EELS, which applies the refund cap after reservoir removal but before the
        // regular/state gas split.
        let gas_refunded: u64 = compute_gas_refunded(vm, ctx_result)?;
        let gas_spent = compute_actual_gas_used(vm, gas_refunded, gas_used_pre_refund)?;

        refund_sender(vm, ctx_result, gas_refunded, gas_spent, gas_used_pre_refund)?;

        pay_coinbase(vm, gas_spent)?;

        delete_self_destruct_accounts(vm)?;

        Ok(())
    }
}

/// EIP-8037: Settle `state_diff_finalized` at the end of a tx.
///
/// Collision and non-success paths: only intrinsic state gas stays charged,
/// so reset finalized diff to the pre-execution intrinsic seed.
/// Success path: snapshot the top frame's accumulated diff (which already
/// contains intrinsic + execution records from `merge_child_state_diff` calls),
/// then apply same-tx-selfdestruct cancellations to remove accounts that were
/// created and then destroyed in the same tx.
///
/// Shared between L1 (`DefaultHook`) and L2 (`L2Hook`) finalize paths.
pub fn settle_state_diff_finalized(vm: &mut VM<'_>, is_success: bool) -> Result<(), VMError> {
    use crate::gas_cost::STATE_BYTES_PER_NEW_ACCOUNT;
    if vm.env.config.fork < Fork::Amsterdam {
        return Ok(());
    }
    if is_success {
        vm.state_diff_finalized = vm.current_call_frame.state_diff.clone();
        // Mirror EELS's deferred SD refund (process_message_call's
        // accounts_to_delete loop): for each account both created and
        // selfdestructed in this tx, refund NEW_ACCOUNT + slot bytes + code_len
        // to the reservoir AND drop the records from the finalized diff. For
        // pre-funded targets where CREATE skipped record_new_account, the
        // +112 still gets refunded (EELS subtracts unconditionally) — track
        // this as a block-state offset since there's no record to remove.
        let selfdestruct_addrs: Vec<Address> = vm.substate.iter_selfdestruct().copied().collect();
        let cpsb = vm.cost_per_state_byte;
        for addr in selfdestruct_addrs {
            if !vm.substate.is_account_created(&addr) {
                continue;
            }
            let mut refund_bytes: u64 = 0;
            if vm.state_diff_finalized.new_accounts.remove(&addr) {
                refund_bytes = refund_bytes.saturating_add(STATE_BYTES_PER_NEW_ACCOUNT);
            } else if vm.create_skipped_pre_existed.contains(&addr) {
                // Pre-funded SD'd: +112 was never recorded, but EELS still
                // refunds it. Track via block-state offset so finalized.bytes()
                // ↓ at block accounting.
                refund_bytes = refund_bytes.saturating_add(STATE_BYTES_PER_NEW_ACCOUNT);
                vm.block_state_offset_bytes = vm
                    .block_state_offset_bytes
                    .saturating_add(STATE_BYTES_PER_NEW_ACCOUNT);
            }
            #[expect(clippy::as_conversions, reason = "filter().count() bounded")]
            let slot_count = vm
                .state_diff_finalized
                .new_storage_slots
                .iter()
                .filter(|(a, _)| *a == addr)
                .count() as u64;
            refund_bytes = refund_bytes.saturating_add(
                slot_count.saturating_mul(crate::gas_cost::STATE_BYTES_PER_STORAGE_SET),
            );
            vm.state_diff_finalized
                .new_storage_slots
                .retain(|(a, _)| *a != addr);
            if let Some(code_len) = vm.state_diff_finalized.code_deposits.remove(&addr) {
                refund_bytes = refund_bytes.saturating_add(code_len);
            }
            if refund_bytes > 0 {
                let refund_gas = refund_bytes.saturating_mul(cpsb);
                // EELS amsterdam/vm/interpreter.py:200 credits ONLY the reservoir
                // for the deferred SD refund (no spill-back to gas_left). User
                // recovers spilled regular gas via `total_remaining = gas_left +
                // reservoir` at fork.py:1057. Crediting gas_remaining here would
                // leave `ctx_result.gas_used` (captured pre-settle) stale and
                // double-credit the user.
                vm.state_gas_reservoir = vm
                    .state_gas_reservoir
                    .checked_add(refund_gas)
                    .ok_or(InternalError::Overflow)?;
            }
        }
    } else {
        vm.state_diff_finalized = vm.state_diff_intrinsic_seed.clone();
    }
    Ok(())
}

pub fn undo_value_transfer(vm: &mut VM<'_>) -> Result<(), VMError> {
    // In a create if Tx was reverted the account won't even exist by this point.
    if !vm.is_create()? {
        vm.decrease_account_balance(vm.current_call_frame.to, vm.current_call_frame.msg_value)?;
    }

    vm.increase_account_balance(vm.env.origin, vm.current_call_frame.msg_value)?;

    Ok(())
}

/// Refunds unused gas to the sender.
///
/// # EIP-7778 Changes
/// - `gas_spent`: Post-refund gas (what the user actually pays)
/// - `gas_used_pre_refund`: Pre-refund gas (for block-level accounting in Amsterdam+)
///
/// For Amsterdam+, the block uses pre-refund gas (`gas_used`) while the user pays post-refund
/// gas (`gas_spent`). Before Amsterdam, both values are the same (post-refund).
pub fn refund_sender(
    vm: &mut VM<'_>,
    ctx_result: &mut ContextResult,
    refunded_gas: u64,
    gas_spent: u64,
    // Currently unused; kept in the signature for call-site symmetry.
    _gas_used_pre_refund: u64,
) -> Result<(), VMError> {
    vm.substate.refunded_gas = refunded_gas;

    // EIP-7778: Separate block vs user gas accounting for Amsterdam+
    // Block header gas_used = max(regular_dimension, state_dimension) per EIP-7778.
    // Receipt cumulative_gas_used = post-refund total (what user pays).
    if vm.env.config.fork >= Fork::Amsterdam {
        // EIP-7623 floor applies to the regular (non-state) gas component only.
        let floor = vm.get_min_gas_used()?;
        // EIP-8037: state_gas derived from the finalized StateDiff (option-2 model).
        // state_diff_finalized is already settled by finalize_execution above:
        //   - success: intrinsic + execution records, same-tx selfdestruct cancellations applied.
        //   - revert/halt: reset to intrinsic seed only.
        let state_gas = vm
            .state_diff_finalized
            .bytes()
            .saturating_sub(vm.block_state_offset_bytes)
            .saturating_mul(vm.cost_per_state_byte);
        // Intrinsic state gas is state_diff_intrinsic_seed.bytes() * cpsb.
        let intrinsic_state_gas = vm
            .state_diff_intrinsic_seed
            .bytes()
            .saturating_mul(vm.cost_per_state_byte);

        // Compute raw consumption from scratch (gas_limit minus gas_remaining)
        // to avoid interference from any reservoir-current subtraction baked
        // into the caller's pre-refund number.
        #[expect(clippy::as_conversions, reason = "gas_remaining is >= 0 here")]
        let gas_remaining = vm.current_call_frame.gas_remaining.max(0) as u64;
        let raw_consumed = vm.env.gas_limit.saturating_sub(gas_remaining);
        let regular_gas = raw_consumed
            .saturating_sub(intrinsic_state_gas)
            .saturating_sub(vm.state_gas_reservoir_initial)
            .saturating_sub(vm.state_gas_spill);
        let effective_regular = regular_gas.max(floor);
        ctx_result.gas_used = effective_regular
            .checked_add(state_gas)
            .ok_or(InternalError::Overflow)?;
        // User pays post-refund gas (with floor)
        ctx_result.gas_spent = gas_spent;
    } else {
        // Pre-Amsterdam: both use post-refund value
        ctx_result.gas_used = gas_spent;
        ctx_result.gas_spent = gas_spent;
    }

    // Return unspent gas to the sender (based on what user pays)
    let gas_to_return = vm
        .env
        .gas_limit
        .checked_sub(gas_spent)
        .ok_or(InternalError::Underflow)?;

    let wei_return_amount = vm
        .env
        .gas_price
        .checked_mul(U256::from(gas_to_return))
        .ok_or(InternalError::Overflow)?;

    vm.increase_account_balance(vm.env.origin, wei_return_amount)?;

    Ok(())
}

// [EIP-3529](https://eips.ethereum.org/EIPS/eip-3529)
pub fn compute_gas_refunded(vm: &VM<'_>, ctx_result: &ContextResult) -> Result<u64, VMError> {
    Ok(vm
        .substate
        .refunded_gas
        .min(ctx_result.gas_used / MAX_REFUND_QUOTIENT))
}

// Calculate actual gas used in the whole transaction. Since Prague there is a base minimum to be consumed.
pub fn compute_actual_gas_used(
    vm: &mut VM<'_>,
    refunded_gas: u64,
    gas_used_without_refunds: u64,
) -> Result<u64, VMError> {
    let exec_gas_consumed = gas_used_without_refunds
        .checked_sub(refunded_gas)
        .ok_or(InternalError::Underflow)?;

    if vm.env.config.fork >= Fork::Prague {
        Ok(exec_gas_consumed.max(vm.get_min_gas_used()?))
    } else {
        Ok(exec_gas_consumed)
    }
}

pub fn pay_coinbase(vm: &mut VM<'_>, gas_to_pay: u64) -> Result<(), VMError> {
    let priority_fee_per_gas = vm
        .env
        .gas_price
        .checked_sub(vm.env.base_fee_per_gas)
        .ok_or(InternalError::Underflow)?;

    let coinbase_fee = U256::from(gas_to_pay)
        .checked_mul(priority_fee_per_gas)
        .ok_or(InternalError::Overflow)?;

    // Per EIP-7928: Coinbase must appear in BAL when there's a user transaction,
    // even if the priority fee is zero. System contract calls have gas_price = 0,
    // so we use this to distinguish them from user transactions.
    if !vm.env.gas_price.is_zero()
        && let Some(recorder) = vm.db.bal_recorder.as_mut()
    {
        recorder.record_touched_address(vm.env.coinbase);
    }

    // Only pay coinbase if there's actually a fee to pay.
    if !coinbase_fee.is_zero() {
        vm.increase_account_balance(vm.env.coinbase, coinbase_fee)?;
    }

    Ok(())
}

// In Cancun the only addresses destroyed are contracts created in this transaction
pub fn delete_self_destruct_accounts(vm: &mut VM<'_>) -> Result<(), VMError> {
    // EIP-7708: Emit Burn logs for accounts with non-zero balance marked for deletion
    // Must emit in lexicographical order of address
    if vm.env.config.fork >= Fork::Amsterdam {
        let mut addresses_with_balance: Vec<(Address, U256)> = vm
            .substate
            .iter_selfdestruct()
            .filter_map(|addr| {
                let balance = vm.db.get_account(*addr).ok()?.info.balance;
                if !balance.is_zero() {
                    Some((*addr, balance))
                } else {
                    None
                }
            })
            .collect();

        // Sort by address (lexicographical order per EIP-7708)
        addresses_with_balance.sort_by_key(|(addr, _)| *addr);

        for (addr, balance) in addresses_with_balance {
            let log = create_burn_log(addr, balance);
            vm.substate.add_log(log);
        }
    }

    // Delete the accounts
    for address in vm.substate.iter_selfdestruct() {
        let account_to_remove = vm.db.get_account_mut(*address)?;
        vm.current_call_frame
            .call_frame_backup
            .backup_account_info(*address, account_to_remove)?;

        *account_to_remove = LevmAccount::default();
        account_to_remove.mark_destroyed();

        // EIP-7928: Clean up BAL for selfdestructed account
        if let Some(recorder) = vm.db.bal_recorder.as_mut() {
            recorder.track_selfdestruct(*address);
        }
    }

    Ok(())
}

pub fn validate_min_gas_limit(vm: &mut VM<'_>) -> Result<(), VMError> {
    // check for gas limit is grater or equal than the minimum required
    let calldata = vm.current_call_frame.calldata.clone();
    let (regular_gas, state_gas) = vm.get_intrinsic_gas()?;
    let intrinsic_gas: u64 = regular_gas
        .checked_add(state_gas)
        .ok_or(ExceptionalHalt::OutOfGas)?;

    if vm.current_call_frame.gas_limit < intrinsic_gas {
        return Err(TxValidationError::IntrinsicGasTooLow.into());
    }

    let fork = vm.env.config.fork;

    // EIP-7976 floor tokens: for the floor arm, all calldata bytes count unweighted.
    // floor_tokens_in_calldata = (zero_bytes + nonzero_bytes) * STANDARD_TOKEN_COST
    // Pre-Amsterdam uses the weighted EIP-7623 formula: (nonzero * 16 + zero * 4) / 4
    let mut tokens_in_calldata: u64 = if fork >= Fork::Amsterdam {
        // EIP-7976: floor tokens = total_bytes * STANDARD_TOKEN_COST (unweighted).
        let total_bytes: u64 = calldata
            .len()
            .try_into()
            .map_err(|_| InternalError::TypeConversion)?;
        total_bytes
            .checked_mul(STANDARD_TOKEN_COST)
            .ok_or(InternalError::Overflow)?
    } else {
        // Pre-Amsterdam: weighted EIP-7623 token count.
        gas_cost::tx_calldata(&calldata)? / STANDARD_TOKEN_COST
    };

    // EIP-7981 (Amsterdam+): access-list data bytes fold into the floor-token count.
    // floor_tokens_in_access_list = access_list_bytes * STANDARD_TOKEN_COST
    // where access_list_bytes = 20 * address_count + 32 * storage_key_count.
    if fork >= Fork::Amsterdam {
        let al_floor_tokens = floor_tokens_in_access_list(vm.tx.access_list());
        tokens_in_calldata = tokens_in_calldata
            .checked_add(al_floor_tokens)
            .ok_or(InternalError::Overflow)?;
    }

    // floor_cost_by_tokens = TX_BASE_COST + total_cost_floor_per_token(fork) * tokens
    // EIP-7976 (Amsterdam+) raises the floor multiplier from 10 to 16.
    let floor_cost_by_tokens = tokens_in_calldata
        .checked_mul(total_cost_floor_per_token(fork))
        .ok_or(InternalError::Overflow)?
        .checked_add(TX_BASE_COST)
        .ok_or(InternalError::Overflow)?;

    // EIP-8037 (Amsterdam+): Regular gas is capped at TX_MAX_GAS_LIMIT — reject if
    // intrinsic regular gas or calldata floor exceeds the cap (no amount of gas_limit
    // can make the TX valid since excess gas_limit becomes state gas reservoir).
    // Must be checked before the floor check so the correct error is returned.
    // NOTE: We use IntrinsicGasTooLow (not TxMaxGasLimitExceeded) intentionally —
    // this matches the EELS exception mapping for this specific case.
    if vm.env.config.fork >= Fork::Amsterdam
        && regular_gas.max(floor_cost_by_tokens) > TX_MAX_GAS_LIMIT_AMSTERDAM
    {
        return Err(TxValidationError::IntrinsicGasTooLow.into());
    }

    if vm.current_call_frame.gas_limit < floor_cost_by_tokens {
        return Err(TxValidationError::IntrinsicGasBelowFloorGasCost.into());
    }

    Ok(())
}

pub fn validate_max_fee_per_blob_gas(
    vm: &mut VM<'_>,
    tx_max_fee_per_blob_gas: U256,
) -> Result<(), VMError> {
    let base_fee_per_blob_gas = vm.env.base_blob_fee_per_gas;
    if tx_max_fee_per_blob_gas < base_fee_per_blob_gas {
        return Err(TxValidationError::InsufficientMaxFeePerBlobGas {
            base_fee_per_blob_gas,
            tx_max_fee_per_blob_gas,
        }
        .into());
    }

    Ok(())
}

pub fn validate_init_code_size(vm: &mut VM<'_>) -> Result<(), VMError> {
    // [EIP-3860] - INITCODE_SIZE_EXCEEDED
    // [EIP-7954] - Amsterdam increases the limit
    let code_size = vm.current_call_frame.calldata.len();
    let max_size = if vm.env.config.fork >= Fork::Amsterdam {
        AMSTERDAM_INIT_CODE_MAX_SIZE
    } else {
        INIT_CODE_MAX_SIZE
    };
    if code_size > max_size && vm.env.config.fork >= Fork::Shanghai {
        return Err(TxValidationError::InitcodeSizeExceeded {
            max_size,
            actual_size: code_size,
        }
        .into());
    }
    Ok(())
}

pub fn validate_sufficient_max_fee_per_gas(vm: &mut VM<'_>) -> Result<(), TxValidationError> {
    if vm.env.tx_max_fee_per_gas.unwrap_or(vm.env.gas_price) < vm.env.base_fee_per_gas {
        return Err(TxValidationError::InsufficientMaxFeePerGas);
    }
    Ok(())
}

pub fn validate_4844_tx(vm: &mut VM<'_>) -> Result<(), VMError> {
    // (11) TYPE_3_TX_PRE_FORK
    if vm.env.config.fork < Fork::Cancun {
        return Err(TxValidationError::Type3TxPreFork.into());
    }

    let blob_hashes = &vm.env.tx_blob_hashes;

    // (12) TYPE_3_TX_ZERO_BLOBS
    if blob_hashes.is_empty() {
        return Err(TxValidationError::Type3TxZeroBlobs.into());
    }

    // (13) TYPE_3_TX_INVALID_BLOB_VERSIONED_HASH
    for blob_hash in blob_hashes {
        let blob_hash = blob_hash.as_bytes();
        if blob_hash
            .first()
            .is_some_and(|first_byte| !VALID_BLOB_PREFIXES.contains(first_byte))
        {
            return Err(TxValidationError::Type3TxInvalidBlobVersionedHash.into());
        }
    }

    // (14) TYPE_3_TX_BLOB_COUNT_EXCEEDED
    let max_blob_count = vm
        .env
        .config
        .blob_schedule
        .max
        .try_into()
        .map_err(|_| InternalError::TypeConversion)?;
    let blob_count = blob_hashes.len();
    if blob_count > max_blob_count {
        return Err(TxValidationError::Type3TxBlobCountExceeded {
            max_blob_count,
            actual_blob_count: blob_count,
        }
        .into());
    }
    if vm.env.config.fork >= Fork::Osaka && blob_count > MAX_BLOB_COUNT_TX {
        return Err(TxValidationError::Type3TxBlobCountExceeded {
            max_blob_count: MAX_BLOB_COUNT_TX,
            actual_blob_count: blob_count,
        }
        .into());
    }

    // (15) TYPE_3_TX_CONTRACT_CREATION
    // NOTE: This will never happen, since the EIP-4844 tx (type 3) does not have a TxKind field
    // only supports an Address which must be non-empty.
    // If a type 3 tx has the field `to` as null (signaling create), it will raise an exception on RLP decoding,
    // it won't reach this point.
    // For more information, please check the following thread:
    // - https://github.com/lambdaclass/ethrex/pull/2425/files/819825516dc633275df56b2886b921061c4d7681#r2035611105
    if vm.is_create()? {
        return Err(TxValidationError::Type3TxContractCreation.into());
    }

    Ok(())
}

pub fn validate_type_4_tx(vm: &mut VM<'_>) -> Result<(), VMError> {
    let Some(auth_list) = vm.tx.authorization_list() else {
        // vm.authorization_list should be Some at this point.
        return Err(InternalError::Custom("Auth list not found".to_string()).into());
    };

    // (16) TYPE_4_TX_PRE_FORK
    if vm.env.config.fork < Fork::Prague {
        return Err(TxValidationError::Type4TxPreFork.into());
    }

    // (17) TYPE_4_TX_CONTRACT_CREATION
    // From the EIP docs: a null destination is not valid.
    // NOTE: This will never happen, since the EIP-7702 tx (type 4) does not have a TxKind field
    // only supports an Address which must be non-empty.
    // If a type 4 tx has the field `to` as null (signaling create), it will raise an exception on RLP decoding,
    // it won't reach this point.
    // For more information, please check the following thread:
    // - https://github.com/lambdaclass/ethrex/pull/2425/files/819825516dc633275df56b2886b921061c4d7681#r2035611105
    if vm.is_create()? {
        return Err(TxValidationError::Type4TxContractCreation.into());
    }

    // (18) TYPE_4_TX_LIST_EMPTY
    // From the EIP docs: The transaction is considered invalid if the length of authorization_list is zero.
    if auth_list.is_empty() {
        return Err(TxValidationError::Type4TxAuthorizationListIsEmpty.into());
    }

    vm.eip7702_set_access_code()
}

pub fn validate_sender(sender_address: Address, code: &Bytes) -> Result<(), VMError> {
    if !code.is_empty() && !code_has_delegation(code)? {
        return Err(TxValidationError::SenderNotEOA(sender_address).into());
    }
    Ok(())
}

pub fn validate_gas_allowance(vm: &mut VM<'_>) -> Result<(), TxValidationError> {
    // System contract calls (EIP-2935, EIP-4788, EIP-7002, EIP-7251) bypass the
    // block-level gas-allowance check — their 30M gas budget is a protocol rule
    // independent of `block_gas_limit`.
    if vm.env.is_system_call {
        return Ok(());
    }
    if vm.env.gas_limit > vm.env.block_gas_limit {
        return Err(TxValidationError::GasAllowanceExceeded {
            block_gas_limit: vm.env.block_gas_limit,
            tx_gas_limit: vm.env.gas_limit,
        });
    }
    Ok(())
}

pub fn validate_sender_balance(vm: &mut VM<'_>, sender_balance: U256) -> Result<(), VMError> {
    if vm.env.disable_balance_check {
        return Ok(());
    }

    // Up front cost is the maximum amount of wei that a user is willing to pay for. Gaslimit * gasprice + value + blob_gas_cost
    let value = vm.current_call_frame.msg_value;

    // blob gas cost = max fee per blob gas * blob gas used
    // https://eips.ethereum.org/EIPS/eip-4844
    let max_blob_gas_cost =
        get_max_blob_gas_price(&vm.env.tx_blob_hashes, vm.env.tx_max_fee_per_blob_gas)?;

    // For the transaction to be valid the sender account has to have a balance >= gas_price * gas_limit + value if tx is type 0 and 1
    // balance >= max_fee_per_gas * gas_limit + value + blob_gas_cost if tx is type 2 or 3
    let gas_fee_for_valid_tx = vm
        .env
        .tx_max_fee_per_gas
        .unwrap_or(vm.env.gas_price)
        .checked_mul(vm.env.gas_limit.into())
        .ok_or(TxValidationError::GasLimitPriceProductOverflow)?;

    let balance_for_valid_tx = gas_fee_for_valid_tx
        .checked_add(value)
        .ok_or(TxValidationError::InsufficientAccountFunds)?
        .checked_add(max_blob_gas_cost)
        .ok_or(TxValidationError::InsufficientAccountFunds)?;

    if sender_balance < balance_for_valid_tx {
        return Err(TxValidationError::InsufficientAccountFunds.into());
    }

    Ok(())
}

pub fn deduct_caller(
    vm: &mut VM<'_>,
    gas_limit_price_product: U256,
    sender_address: Address,
) -> Result<(), VMError> {
    if vm.env.disable_balance_check {
        return Ok(());
    }

    // Up front cost is the maximum amount of wei that a user is willing to pay for. Gaslimit * gasprice + value + blob_gas_cost
    let value = vm.current_call_frame.msg_value;

    let blob_gas_cost = calculate_blob_gas_cost(
        &vm.env.tx_blob_hashes,
        vm.env.block_excess_blob_gas,
        &vm.env.config,
    )?;

    // The real cost to deduct is calculated as effective_gas_price * gas_limit + value + blob_gas_cost
    let up_front_cost = gas_limit_price_product
        .checked_add(value)
        .ok_or(TxValidationError::InsufficientAccountFunds)?
        .checked_add(blob_gas_cost)
        .ok_or(TxValidationError::InsufficientAccountFunds)?;
    // There is no error specified for overflow in up_front_cost
    // in ef_tests. We went for "InsufficientAccountFunds" simply
    // because if the upfront cost is bigger than U256, then,
    // technically, the sender will not be able to pay it.

    vm.decrease_account_balance(sender_address, up_front_cost)
        .map_err(|_| TxValidationError::InsufficientAccountFunds)?;

    Ok(())
}

/// Transfer msg_value to transaction recipient
pub fn transfer_value(vm: &mut VM<'_>) -> Result<(), VMError> {
    if !vm.is_create()? {
        let value = vm.current_call_frame.msg_value;
        let to = vm.current_call_frame.to;

        vm.increase_account_balance(to, value)?;

        // EIP-7708: Emit transfer log for nonzero-value transactions to DIFFERENT accounts
        // Self-transfers (origin == to) should NOT emit a log per the EIP spec
        let from = vm.env.origin;
        if vm.env.config.fork >= Fork::Amsterdam && !value.is_zero() && from != to {
            let log = create_eth_transfer_log(from, to, value);
            vm.substate.add_log(log);
        }
    }
    Ok(())
}

/// Sets bytecode and code_address to CallFrame
pub fn set_bytecode_and_code_address(vm: &mut VM<'_>) -> Result<(), VMError> {
    // Get bytecode and code_address for assigning those values to the callframe.
    let (bytecode, code_address) = if vm.is_create()? {
        // Here bytecode is the calldata and the code_address is just the created contract address.
        let calldata = std::mem::take(&mut vm.current_call_frame.calldata);
        (
            // SAFETY: we don't need the hash for the initcode
            Code::from_bytecode_unchecked(calldata, H256::zero()),
            vm.current_call_frame.to,
        )
    } else {
        // Here bytecode and code_address could be either from the account or from the delegated account.
        let to = vm.current_call_frame.to;

        // Record tx.to as touched in BAL (the target of message call transaction)
        if let Some(recorder) = vm.db.bal_recorder.as_mut() {
            recorder.record_touched_address(to);
        }

        let (is_delegation, _eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(vm.db, &mut vm.substate, to)?;

        // If EIP-7702 delegation, also record the delegation target (code source) in BAL
        if is_delegation && let Some(recorder) = vm.db.bal_recorder.as_mut() {
            recorder.record_touched_address(code_address);
        }

        (bytecode, code_address)
    };

    // Assign code and code_address to callframe
    vm.current_call_frame.code_address = code_address;
    vm.current_call_frame.set_code(bytecode)?;

    Ok(())
}
