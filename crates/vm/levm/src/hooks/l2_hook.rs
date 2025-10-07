use crate::{
    call_frame::CallFrameBackup,
    errors::{ContextResult, InternalError, TxValidationError},
    hooks::{DefaultHook, default_hook, hook::Hook},
    opcodes::Opcode,
    utils::get_account_diffs_in_tx,
    vm::VM,
};

use ethrex_common::{
    Address, H160, U256,
    constants::GAS_PER_BLOB,
    types::{
        SAFE_BYTES_PER_BLOB,
        account_diff::get_accounts_diff_size,
        fee_config::{FeeConfig, L1FeeConfig, OperatorFeeConfig},
    },
};

pub const COMMON_BRIDGE_L2_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xff,
]);

pub struct L2Hook {
    pub fee_config: FeeConfig,
    pub pre_execution_backup: CallFrameBackup,
}

impl Hook for L2Hook {
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), crate::errors::VMError> {
        if !vm.env.is_privileged {
            DefaultHook.prepare_execution(vm)?;
            // Different from L1:
            // Operator fee is deducted from the sender before execution
            deduct_operator_fee(vm, &self.fee_config.operator_fee_config)?;

            // We need to backup the callframe to calculate the state diff later
            self.pre_execution_backup = vm.current_call_frame.call_frame_backup.clone();
            return Ok(());
        }

        let sender_address = vm.env.origin;
        let sender_balance = vm.db.get_account(sender_address)?.info.balance;

        let mut tx_should_fail = false;

        // The bridge is allowed to mint ETH.
        // This is done by not decreasing it's balance when it's the source of a transfer.
        // For other privileged transactions, insufficient balance can't cause an error
        // since they must always be accepted, and an error would mark them as invalid
        // Instead, we make them revert by inserting a revert2
        if sender_address != COMMON_BRIDGE_L2_ADDRESS {
            let value = vm.current_call_frame.msg_value;
            if value > sender_balance {
                tx_should_fail = true;
            } else {
                // This should never fail, since we just checked the balance is enough.
                vm.decrease_account_balance(sender_address, value)
                    .map_err(|_| {
                        InternalError::Custom(
                            "Insufficient funds in privileged transaction".to_string(),
                        )
                    })?;
            }
        }

        // if fork > prague: default_hook::validate_min_gas_limit
        // NOT CHECKED: the l1 makes spamming privileged transactions not economical

        // (1) GASLIMIT_PRICE_PRODUCT_OVERFLOW
        // NOT CHECKED: privileged transactions do not pay for gas

        // (2) INSUFFICIENT_MAX_FEE_PER_BLOB_GAS
        // NOT CHECKED: the blob price does not matter, privileged transactions do not support blobs

        // (4) INSUFFICIENT_MAX_FEE_PER_GAS
        // NOT CHECKED: privileged transactions do not pay for gas, the gas price is irrelevant

        // (5) INITCODE_SIZE_EXCEEDED
        // NOT CHECKED: privileged transactions can't be of "create" type

        // (6) INTRINSIC_GAS_TOO_LOW
        // CHANGED: the gas should be charged, but the transaction shouldn't error
        if vm.add_intrinsic_gas().is_err() {
            tx_should_fail = true;
        }

        // (7) NONCE_IS_MAX
        // NOT CHECKED: privileged transactions don't use the account nonce

        // (8) PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS
        // NOT CHECKED: privileged transactions do not pay for gas, the gas price is irrelevant

        // (9) SENDER_NOT_EOA
        // NOT CHECKED: contracts can also send privileged transactions

        // (10) GAS_ALLOWANCE_EXCEEDED
        // CHECKED: we don't want to exceed block limits
        default_hook::validate_gas_allowance(vm)?;

        // Transaction is type 3 if tx_max_fee_per_blob_gas is Some
        // NOT CHECKED: privileged transactions are not type 3

        // Transaction is type 4 if authorization_list is Some
        // NOT CHECKED: privileged transactions are not type 4

        if tx_should_fail {
            // If the transaction failed some validation, but it must still be included
            // To prevent it from taking effect, we force it to revert
            vm.current_call_frame.msg_value = U256::zero();
            vm.current_call_frame
                .set_code(vec![Opcode::INVALID.into()].into())?;
            return Ok(());
        }

        default_hook::transfer_value(vm)?;

        default_hook::set_bytecode_and_code_address(vm)?;

        Ok(())
    }

    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        ctx_result: &mut ContextResult,
    ) -> Result<(), crate::errors::VMError> {
        if !vm.env.is_privileged {
            DefaultHook.finalize_execution(vm, ctx_result)?;
            // Different from L1:

            // Base fee is not burned
            pay_base_fee_vault(vm, ctx_result.gas_used, self.fee_config.base_fee_vault)?;

            // Operator fee is paid to the chain operator
            pay_operator_fee(vm, self.fee_config.operator_fee_config)?;

            //
            pay_l1_fee(
                vm,
                std::mem::take(&mut self.pre_execution_backup),
                self.fee_config.l1_fee_config,
            )?;

            return Ok(());
        }

        if !ctx_result.is_success() && vm.env.origin != COMMON_BRIDGE_L2_ADDRESS {
            default_hook::undo_value_transfer(vm)?;
        }

        // Even if privileged transactions themselves can't create
        // They can call contracts that use CREATE/CREATE2
        default_hook::delete_self_destruct_accounts(vm)?;

        Ok(())
    }
}

fn deduct_operator_fee(
    vm: &mut VM<'_>,
    operator_fee_config: &Option<OperatorFeeConfig>,
) -> Result<(), crate::errors::VMError> {
    let Some(fee_config) = operator_fee_config else {
        // No operator fee configured, operator fee is not paid
        return Ok(());
    };
    let sender_address = vm.env.origin;

    vm.decrease_account_balance(sender_address, fee_config.operator_fee)
        .map_err(|_| TxValidationError::InsufficientAccountFunds)?;
    Ok(())
}

fn pay_base_fee_vault(
    vm: &mut VM<'_>,
    gas_to_pay: u64,
    base_fee_vault: Option<Address>,
) -> Result<(), crate::errors::VMError> {
    let Some(base_fee_vault) = base_fee_vault else {
        // No base fee vault configured, base fee is effectively burned
        return Ok(());
    };

    let base_fee = U256::from(gas_to_pay)
        .checked_mul(vm.env.base_fee_per_gas)
        .ok_or(InternalError::Overflow)?;

    vm.increase_account_balance(base_fee_vault, base_fee)?;
    Ok(())
}

fn pay_operator_fee(
    vm: &mut VM<'_>,
    operator_fee_config: Option<OperatorFeeConfig>,
) -> Result<(), crate::errors::VMError> {
    let Some(fee_config) = operator_fee_config else {
        // No operator fee configured, operator fee is not paid
        return Ok(());
    };

    vm.increase_account_balance(fee_config.operator_fee_vault, fee_config.operator_fee)?;
    Ok(())
}

fn pay_l1_fee(
    vm: &mut VM<'_>,
    pre_execution_backup: CallFrameBackup,
    l1_fee_config: Option<L1FeeConfig>,
) -> Result<(), crate::errors::VMError> {
    let Some(fee_config) = l1_fee_config else {
        // No l1 fee configured, l1 fee is not paid
        return Ok(());
    };

    let mut execution_backup = vm.current_call_frame.call_frame_backup.clone();
    execution_backup.extend(pre_execution_backup);
    let account_diffs_in_tx = get_account_diffs_in_tx(vm.db, execution_backup)?;
    let account_diffs_size = get_accounts_diff_size(&account_diffs_in_tx)
        .map_err(|e| InternalError::Custom(format!("Failed to get account diffs size: {}", e)))?;

    let l1_fee_per_blob: U256 = fee_config
        .l1_fee_per_blob_gas
        .checked_mul(GAS_PER_BLOB.into())
        .ok_or(InternalError::Overflow)?;

    let l1_fee_per_blob_byte = l1_fee_per_blob
        .checked_div(U256::from(SAFE_BYTES_PER_BLOB))
        .ok_or(InternalError::DivisionByZero)?;

    let l1_fee = l1_fee_per_blob_byte
        .checked_mul(U256::from(account_diffs_size))
        .ok_or(InternalError::Overflow)?;

    let sender_address = vm.env.origin;

    vm.decrease_account_balance(sender_address, l1_fee)
        .map_err(|_| TxValidationError::InsufficientAccountFunds)?;

    vm.increase_account_balance(fee_config.l1_fee_vault, l1_fee)?;
    Ok(())
}
