use crate::{
    errors::{ContextResult, InternalError, TxValidationError, VMError},
    hooks::{default_hook, hook::Hook},
    vm::VM,
};

use ethrex_common::{types::Fork, Address, U256};

pub struct L2Hook {
    pub recipient: Option<Address>,
}

impl Hook for L2Hook {
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), crate::errors::VMError> {
        if vm.env.is_privileged {
            let Some(recipient) = self.recipient else {
                return Err(InternalError::RecipientNotFoundForPrivilegedTransaction.into());
            };
            vm.increase_account_balance(recipient, vm.current_call_frame()?.msg_value)?;
            vm.current_call_frame_mut()?.msg_value = U256::from(0);
        }

        let sender_address = vm.env.origin;
        let (sender_balance, sender_nonce) = {
            let sender_account = vm.db.get_account(sender_address)?;
            (sender_account.info.balance, sender_account.info.nonce)
        };

        if vm.env.config.fork >= Fork::Prague {
            default_hook::validate_min_gas_limit(vm)?;
        }

        if !vm.env.is_privileged {
            // (1) GASLIMIT_PRICE_PRODUCT_OVERFLOW
            let gaslimit_price_product = vm
                .env
                .gas_price
                .checked_mul(vm.env.gas_limit.into())
                .ok_or(TxValidationError::GasLimitPriceProductOverflow)?;

            default_hook::validate_sender_balance(vm, sender_balance)?;

            // (3) INSUFFICIENT_ACCOUNT_FUNDS
            default_hook::deduct_caller(vm, gaslimit_price_product, sender_address)?;

            // (7) NONCE_IS_MAX
            vm.increment_account_nonce(sender_address)
                .map_err(|_| TxValidationError::NonceIsMax)?;

            // check for nonce mismatch
            if sender_nonce != vm.env.tx_nonce {
                return Err(TxValidationError::NonceMismatch {
                    expected: sender_nonce,
                    actual: vm.env.tx_nonce,
                }
                .into());
            }

            // (9) SENDER_NOT_EOA
            default_hook::validate_sender(vm.db.get_account(sender_address)?)?;
        }

        // (2) INSUFFICIENT_MAX_FEE_PER_BLOB_GAS
        if let Some(tx_max_fee_per_blob_gas) = vm.env.tx_max_fee_per_blob_gas {
            default_hook::validate_max_fee_per_blob_gas(vm, tx_max_fee_per_blob_gas)?;
        }

        // (4) INSUFFICIENT_MAX_FEE_PER_GAS
        default_hook::validate_sufficient_max_fee_per_gas(vm)?;

        // (5) INITCODE_SIZE_EXCEEDED
        if vm.is_create()? {
            default_hook::validate_init_code_size(vm)?;
        }

        // (6) INTRINSIC_GAS_TOO_LOW
        vm.add_intrinsic_gas()?;

        // (8) PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS
        if let (Some(tx_max_priority_fee), Some(tx_max_fee_per_gas)) = (
            vm.env.tx_max_priority_fee_per_gas,
            vm.env.tx_max_fee_per_gas,
        ) {
            if tx_max_priority_fee > tx_max_fee_per_gas {
                return Err(TxValidationError::PriorityGreaterThanMaxFeePerGas.into());
            }
        }

        // (10) GAS_ALLOWANCE_EXCEEDED
        default_hook::validate_gas_allowance(vm)?;

        // Transaction is type 3 if tx_max_fee_per_blob_gas is Some
        if vm.env.tx_max_fee_per_blob_gas.is_some() {
            default_hook::validate_4844_tx(vm)?;
        }

        // [EIP-7702]: https://eips.ethereum.org/EIPS/eip-7702
        // Transaction is type 4 if authorization_list is Some
        if vm.tx.authorization_list().is_some() {
            default_hook::validate_type_4_tx(vm)?;
        }

        default_hook::transfer_value_if_applicable(vm)?;

        default_hook::set_bytecode_and_code_address(vm)?;

        Ok(())
    }

    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        ctx_result: &mut ContextResult,
    ) -> Result<(), crate::errors::VMError> {
        if !ctx_result.is_success() {
            if vm.env.is_privileged {
                undo_value_transfer(vm)?;
            } else {
                default_hook::undo_value_transfer(vm)?;
            }
            vm.increase_account_balance(vm.env.origin, vm.current_call_frame()?.msg_value)?;
        }

        // 2. Return unused gas + gas refunds to the sender.

        if !vm.env.is_privileged {
            let gas_refunded = default_hook::compute_gas_refunded(vm, ctx_result)?;
            let actual_gas_used =
                default_hook::compute_actual_gas_used(vm, gas_refunded, ctx_result.gas_used)?;
            default_hook::refund_sender(vm, ctx_result, gas_refunded, actual_gas_used)?;
            default_hook::pay_coinbase(vm, actual_gas_used)?;
        }

        default_hook::delete_self_destruct_accounts(vm)?;

        Ok(())
    }
}

pub fn undo_value_transfer(vm: &mut VM<'_>) -> Result<(), VMError> {
    if !vm.is_create()? {
        vm.decrease_account_balance(
            vm.current_call_frame()?.to,
            vm.current_call_frame()?.msg_value,
        )?;
    }
    Ok(())
}
