use ethrex_common::{types::Fork, Address, U256};

use crate::{
    db::cache::remove_account,
    errors::{InternalError, TxResult, TxValidationError, VMError},
    hooks::default_hook,
    utils::{
        add_intrinsic_gas, get_account, get_valid_jump_destinations, has_delegation,
        increase_account_balance,
    },
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
        increase_account_balance(vm.db, self.recipient, initial_call_frame.msg_value)?;

        initial_call_frame.msg_value = U256::from(0);

        if vm.env.config.fork >= Fork::Prague {
            default_hook::check_min_gas_limit(vm, initial_call_frame)?;
        }

        // (2) INSUFFICIENT_MAX_FEE_PER_BLOB_GAS
        if let Some(tx_max_fee_per_blob_gas) = vm.env.tx_max_fee_per_blob_gas {
            default_hook::check_max_fee_per_blob_gas(vm, tx_max_fee_per_blob_gas)?;
        }

        // (4) INSUFFICIENT_MAX_FEE_PER_GAS
        default_hook::validate_sufficient_max_fee_per_gas(vm)?;

        // (5) INITCODE_SIZE_EXCEEDED
        if vm.is_create() {
            default_hook::validate_init_code_size(vm, initial_call_frame)?;
        }

        // (6) INTRINSIC_GAS_TOO_LOW
        add_intrinsic_gas(vm, initial_call_frame)?;

        // (7) NONCE_IS_MAX
        if !is_privilege_tx {
            increment_account_nonce(vm.db, sender_address)
                .map_err(|_| VMError::TxValidation(TxValidationError::NonceIsMax))?;

            // check for nonce mismatch
            if sender_account.info.nonce != vm.env.tx_nonce {
                return Err(VMError::TxValidation(TxValidationError::NonceMismatch));
            }
        }

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

        if !is_privilege_tx {
            // (9) SENDER_NOT_EOA
            default_hook::validate_sender(&sender_account)?;
        }

        // (10) GAS_ALLOWANCE_EXCEEDED
        if vm.env.gas_limit > vm.env.block_gas_limit {
            return Err(VMError::TxValidation(
                TxValidationError::GasAllowanceExceeded,
            ));
        }

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
        // POST-EXECUTION Changes
        let receiver_address = initial_call_frame.to;

        // 1. Undo value transfer if the transaction has reverted
        if let TxResult::Revert(_) = report.result {
            let existing_account = get_account(vm.db, receiver_address)?; //TO Account

            if has_delegation(&existing_account.info)? {
                // This is the case where the "to" address and the
                // "signer" address are the same. We are setting the code
                // and sending some balance to the "to"/"signer"
                // address.
                // See https://eips.ethereum.org/EIPS/eip-7702#behavior (last sentence).

                // If transaction execution results in failure (any
                // exceptional condition or code reverting), setting
                // delegation designations is not rolled back.
            } else {
                // We remove the receiver account from the cache, like nothing changed in it's state.
                remove_account(&mut vm.db.cache, &receiver_address);
            }
        }

        // 2. Return unused gas + gas refunds to the sender.
        let mut consumed_gas = report.gas_used;
        // [EIP-3529](https://eips.ethereum.org/EIPS/eip-3529)
        let quotient = if vm.env.config.fork < Fork::London {
            MAX_REFUND_QUOTIENT_PRE_LONDON
        } else {
            MAX_REFUND_QUOTIENT
        };
        let mut refunded_gas = report.gas_refunded.min(
            consumed_gas
                .checked_div(quotient)
                .ok_or(VMError::Internal(InternalError::UndefinedState(-1)))?,
        );
        // "The max refundable proportion of gas was reduced from one half to one fifth by EIP-3529 by Buterin and Swende [2021] in the London release"
        report.gas_refunded = refunded_gas;

        if vm.env.config.fork >= Fork::Prague {
            let floor_gas_price = vm.get_min_gas_used(initial_call_frame)?;
            let execution_gas_used = consumed_gas.saturating_sub(refunded_gas);
            if floor_gas_price > execution_gas_used {
                consumed_gas = floor_gas_price;
                refunded_gas = 0;
            }
        }

        // 3. Pay coinbase fee
        let gas_to_pay_coinbase = consumed_gas
            .checked_sub(refunded_gas)
            .ok_or(VMError::Internal(InternalError::UndefinedState(2)))?;

        default_hook::pay_coinbase_fee(vm, gas_to_pay_coinbase)?;

        // 4. Destruct addresses in vm.estruct set.
        default_hook::delete_self_destruct_accounts(vm)?;

        Ok(())
    }
}
