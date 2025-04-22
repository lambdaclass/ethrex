use ethrex_common::{types::Fork, Address, U256};

use crate::{
    db::cache::{insert_account, remove_account},
    errors::{InternalError, TxResult, TxValidationError, VMError},
    hooks::default_hook,
    utils::{
        add_intrinsic_gas, decrease_account_balance, get_account, get_valid_jump_destinations,
        has_delegation, increase_account_balance, increment_account_nonce,
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

        // POST-EXECUTION Changes
        let sender_address = initial_call_frame.msg_sender;
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
                if !is_privilege_tx {
                    decrease_account_balance(
                        vm.db,
                        receiver_address,
                        initial_call_frame.msg_value,
                    )?;
                }
            } else if !is_privilege_tx {
                // If the receiver of the transaction was in the cache before the transaction we restore it's state,
                // but if it wasn't then we remove the account from cache like nothing happened.
                if let Some(receiver_account) = vm.cache_backup.get(&receiver_address) {
                    insert_account(&mut vm.db.cache, receiver_address, receiver_account.clone());
                } else {
                    remove_account(&mut vm.db.cache, &receiver_address);
                }
            } else {
                // We remove the receiver account from the cache, like nothing changed in it's state.
                remove_account(&mut vm.db.cache, &receiver_address);
            }

            if !is_privilege_tx {
                increase_account_balance(vm.db, sender_address, initial_call_frame.msg_value)?;
            }
        }

        // 2. Return unused gas + gas refunds to the sender.
        let mut consumed_gas = report.gas_used;
        // [EIP-3529](https://eips.ethereum.org/EIPS/eip-3529)
        let refund_quotient = if vm.env.config.fork < Fork::London {
            MAX_REFUND_QUOTIENT_PRE_LONDON
        } else {
            MAX_REFUND_QUOTIENT
        };
        let mut refunded_gas = report.gas_refunded.min(
            consumed_gas
                .checked_div(refund_quotient)
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
