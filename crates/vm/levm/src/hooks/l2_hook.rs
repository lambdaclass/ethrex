use crate::{
    call_frame::CallFrame,
    db::cache::remove_account,
    errors::{ExecutionReport, InternalError, TxResult, TxValidationError, VMError},
    hooks::{default_hook, hook::Hook},
    utils::{
        add_intrinsic_gas, get_account, get_valid_jump_destinations, has_delegation,
        increase_account_balance, increment_account_nonce,
    },
    vm::VM,
};

use ethrex_common::{types::Fork, Address, U256};

pub struct L2Hook {
    pub recipient: Option<Address>,
}

impl Hook for L2Hook {
    fn prepare_execution(
        &self,
        vm: &mut crate::vm::VM<'_>,
        initial_call_frame: &mut crate::call_frame::CallFrame,
    ) -> Result<(), crate::errors::VMError> {
        if vm.env.is_privilege {
            let Some(recipient) = self.recipient else {
                return Err(VMError::Internal(
                    InternalError::RecipientNotFoundForPrivilegeTransaction,
                ));
            };
            increase_account_balance(vm.db, recipient, initial_call_frame.msg_value)?;
            initial_call_frame.msg_value = U256::from(0);
        }

        let sender_address = vm.env.origin;
        let sender_account = get_account(vm.db, sender_address)?;

        if vm.env.config.fork >= Fork::Prague {
            default_hook::validate_min_gas_limit(vm, initial_call_frame)?;
        }

        if !vm.env.is_privilege {
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
        } else if !vm.env.is_privilege {
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
        if let TxResult::Revert(_) = report.result {
            if vm.env.is_privilege {
                undo_value_transfer(vm, initial_call_frame)?;
            } else {
                default_hook::undo_value_transfer(vm, initial_call_frame)?;
            }
        }

        if vm.env.is_privilege {
            let gas_to_pay_coinbase = compute_coinbase_fee(vm, initial_call_frame, report)?;
            default_hook::pay_coinbase(vm, gas_to_pay_coinbase)?;
        } else {
            let gas_refunded = default_hook::compute_gas_refunded(vm, report)?;
            let actual_gas_used = default_hook::compute_actual_gas_used(
                vm,
                initial_call_frame,
                gas_refunded,
                report.gas_used,
            )?;
            default_hook::refund_sender(
                vm,
                initial_call_frame,
                report,
                gas_refunded,
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

pub fn compute_coinbase_fee(
    vm: &mut VM<'_>,
    initial_call_frame: &CallFrame,
    report: &mut ExecutionReport,
) -> Result<u64, VMError> {
    let mut gas_refunded = default_hook::compute_gas_refunded(vm, report)?;
    let mut gas_consumed = report.gas_used;

    report.gas_refunded = gas_refunded;

    if vm.env.config.fork >= Fork::Prague {
        let floor_gas_price = vm.get_min_gas_used(initial_call_frame)?;
        let execution_gas_used = gas_consumed.saturating_sub(gas_refunded);
        if floor_gas_price > execution_gas_used {
            gas_consumed = floor_gas_price;
            gas_refunded = 0;
        }
    }

    gas_consumed
        .checked_sub(gas_refunded)
        .ok_or(VMError::Internal(InternalError::UndefinedState(2)))
}
