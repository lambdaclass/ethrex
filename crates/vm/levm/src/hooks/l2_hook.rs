use ethrex_common::{types::Fork, Address, U256};

use crate::{
    call_frame::CallFrame,
    db::cache::remove_account,
    errors::{ExecutionReport, InternalError, TxResult, TxValidationError, VMError},
    hooks::default_hook,
    utils::{
        add_intrinsic_gas, get_account, get_valid_jump_destinations, has_delegation,
        increase_account_balance, increment_account_nonce,
    },
    vm::VM,
};

use super::{
    default_hook::{MAX_REFUND_QUOTIENT, MAX_REFUND_QUOTIENT_PRE_LONDON},
    hook::Hook,
};

pub struct L2Hook {
    pub recipient: Address,
}

impl Hook for L2Hook {
    fn prepare_execution(
        &self,
        vm: &mut crate::vm::VM<'_>,
        initial_call_frame: &mut crate::call_frame::CallFrame,
    ) -> Result<(), crate::errors::VMError> {
        // FIXME: L2Hook should behave like the DefaultHook but with extra or
        // less steps. Currently it's not the case since it is hardcoded to
        // always be used for Privilege txs, specifically deposit txs.
        // This efforts will be done in two steps:
        // 1. Refactoring L2Hook to be a DefaultHook with some extra steps.
        // 2. Adding logic to detect privilege transactions since now it is not
        // possible.
        // As part of the first step we will continue using the L2Hook for
        // privilege transactions, hence the hardcoded is_privilege_tx.
        let is_privilege_tx = true;

        if is_privilege_tx {
            increase_account_balance(vm.db, self.recipient, initial_call_frame.msg_value)?;
            initial_call_frame.msg_value = U256::from(0);
        }

        let sender_address = vm.env.origin;
        let sender_account = get_account(vm.db, sender_address)?;

        if vm.env.config.fork >= Fork::Prague {
            default_hook::validate_min_gas_limit(vm, initial_call_frame)?;
        }

        if !is_privilege_tx {
            // (1) GASLIMIT_PRICE_PRODUCT_OVERFLOW
            let gaslimit_price_product = vm
                .env
                .gas_price
                .checked_mul(vm.env.gas_limit.into())
                .ok_or(VMError::TxValidation(
                    TxValidationError::GasLimitPriceProductOverflow,
                ))?;

            default_hook::validate_sender_balance(vm, initial_call_frame, &sender_account)?;

            // (3) INSUFFICIENT_ACCOUNT_FUNDS
            default_hook::deduct_caller(
                vm,
                initial_call_frame,
                gaslimit_price_product,
                sender_address,
            )?;

            // (7) NONCE_IS_MAX
            increment_account_nonce(vm.db, sender_address)
                .map_err(|_| VMError::TxValidation(TxValidationError::NonceIsMax))?;

            // check for nonce mismatch
            if sender_account.info.nonce != vm.env.tx_nonce {
                return Err(VMError::TxValidation(TxValidationError::NonceMismatch));
            }

            // (9) SENDER_NOT_EOA
            default_hook::validate_sender(&sender_account)?;
        }

        // (2) INSUFFICIENT_MAX_FEE_PER_BLOB_GAS
        if let Some(tx_max_fee_per_blob_gas) = vm.env.tx_max_fee_per_blob_gas {
            default_hook::validate_max_fee_per_blob_gas(vm, tx_max_fee_per_blob_gas)?;
        }

        // (4) INSUFFICIENT_MAX_FEE_PER_GAS
        default_hook::validate_sufficient_max_fee_per_gas(vm)?;

        // (5) INITCODE_SIZE_EXCEEDED
        if vm.is_create() {
            default_hook::validate_init_code_size(vm, initial_call_frame)?;
        }

        // (6) INTRINSIC_GAS_TOO_LOW
        add_intrinsic_gas(vm, initial_call_frame)?;

        // (8) PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS
        if let (Some(tx_max_priority_fee), Some(tx_max_fee_per_gas)) = (
            vm.env.tx_max_priority_fee_per_gas,
            vm.env.tx_max_fee_per_gas,
        ) {
            if tx_max_priority_fee > tx_max_fee_per_gas {
                return Err(VMError::TxValidation(
                    TxValidationError::PriorityGreaterThanMaxFeePerGas,
                ));
            }
        }

        // (10) GAS_ALLOWANCE_EXCEEDED
        default_hook::validate_gas_allowance(vm)?;

        // Transaction is type 3 if tx_max_fee_per_blob_gas is Some
        if vm.env.tx_max_fee_per_blob_gas.is_some() {
            default_hook::validate_type_3_tx(vm)?;
        }

        // [EIP-7702]: https://eips.ethereum.org/EIPS/eip-7702
        // Transaction is type 4 if authorization_list is Some
        if vm.authorization_list.is_some() {
            default_hook::validate_type_4_tx(vm, initial_call_frame)?;
        }

        if vm.is_create() {
            // Assign bytecode to context and empty calldata
            initial_call_frame.bytecode = std::mem::take(&mut initial_call_frame.calldata);
            initial_call_frame.valid_jump_destinations =
                get_valid_jump_destinations(&initial_call_frame.bytecode).unwrap_or_default();
        } else if !is_privilege_tx {
            // Transfer value to receiver
            // It's here to avoid storing the "to" address in the cache before eip7702_set_access_code() step 7).
            increase_account_balance(vm.db, initial_call_frame.to, initial_call_frame.msg_value)?;
        }
        Ok(())
    }

    fn finalize_execution(
        &self,
        vm: &mut crate::vm::VM<'_>,
        initial_call_frame: &crate::call_frame::CallFrame,
        report: &mut crate::errors::ExecutionReport,
    ) -> Result<(), crate::errors::VMError> {
        // FIXME: L2Hook should behave like the DefaultHook but with extra or
        // less steps. Currently it's not the case since it is hardcoded to
        // always be used for Privilege txs, specifically deposit txs.
        // This efforts will be done in two steps:
        // 1. Refactoring L2Hook to be a DefaultHook with some extra steps.
        // 2. Adding logic to detect privilege transactions since now it is not
        // possible.
        // As part of the first step we will continue using the L2Hook for
        // privilege transactions, hence the hardcoded is_privilege_tx.
        let is_privilege_tx = true;

        if let TxResult::Revert(_) = report.result {
            if is_privilege_tx {
                undo_value_transfer(vm, initial_call_frame)?;
            } else {
                default_hook::undo_value_transfer(vm, initial_call_frame)?;
            }
        }

        if is_privilege_tx {
            let mut gas_consumed = report.gas_used;
            let gas_refunded = refund_sender(vm, initial_call_frame, &mut gas_consumed, report)?;
            let gas_to_pay_coinbase = gas_consumed
                .checked_sub(gas_refunded)
                .ok_or(VMError::Internal(InternalError::UndefinedState(2)))?;
            default_hook::pay_coinbase(vm, gas_to_pay_coinbase)?;
        } else {
            let refunded_gas = default_hook::compute_refunded_gas(vm, report)?;
            let actual_gas_used = default_hook::compute_actual_gas_used(
                vm,
                initial_call_frame,
                refunded_gas,
                report.gas_used,
            )?;
            default_hook::refund_sender(
                vm,
                initial_call_frame,
                report,
                refunded_gas,
                actual_gas_used,
            )?;

            default_hook::pay_coinbase(vm, actual_gas_used)?;
        }

        default_hook::delete_self_destruct_accounts(vm)?;

        Ok(())
    }
}

pub fn undo_value_transfer(vm: &mut VM<'_>, initial_call_frame: &CallFrame) -> Result<(), VMError> {
    let receiver_address = initial_call_frame.to;
    let existing_account = get_account(vm.db, receiver_address)?;
    if !has_delegation(&existing_account.info)? {
        remove_account(&mut vm.db.cache, &receiver_address);
    }
    Ok(())
}

pub fn refund_sender(
    vm: &mut VM<'_>,
    initial_call_frame: &CallFrame,
    gas_consumed: &mut u64,
    report: &mut ExecutionReport,
) -> Result<u64, VMError> {
    // [EIP-3529](https://eips.ethereum.org/EIPS/eip-3529)
    let refund_quotient = if vm.env.config.fork < Fork::London {
        MAX_REFUND_QUOTIENT_PRE_LONDON
    } else {
        MAX_REFUND_QUOTIENT
    };
    let mut gas_refunded = report.gas_refunded.min(
        gas_consumed
            .checked_div(refund_quotient)
            .ok_or(VMError::Internal(InternalError::UndefinedState(-1)))?,
    );
    // "The max refundable proportion of gas was reduced from one half to one fifth by EIP-3529 by Buterin and Swende [2021] in the London release"
    report.gas_refunded = gas_refunded;

    if vm.env.config.fork >= Fork::Prague {
        let floor_gas_price = vm.get_min_gas_used(initial_call_frame)?;
        let execution_gas_used = gas_consumed.saturating_sub(gas_refunded);
        if floor_gas_price > execution_gas_used {
            *gas_consumed = floor_gas_price;
            gas_refunded = 0;
        }
    }
    Ok(gas_refunded)
}
