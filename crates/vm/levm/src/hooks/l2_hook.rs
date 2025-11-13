use crate::{
    errors::{ContextResult, InternalError, TxValidationError},
    hooks::{
        DefaultHook,
        default_hook::{
            self, compute_actual_gas_used, compute_gas_refunded, delete_self_destruct_accounts,
            pay_coinbase, refund_sender, undo_value_transfer,
        },
        hook::Hook,
    },
    opcodes::Opcode,
    vm::VM,
};

use bytes::Bytes;
use ethrex_common::{
    Address, H160, H256, U256,
    constants::GAS_PER_BLOB,
    types::{
        Code,
        {
            SAFE_BYTES_PER_BLOB,
            fee_config::{FeeConfig, L1FeeConfig, OperatorFeeConfig},
        },
    },
};
use ethrex_rlp::encode::RLPEncode;

pub const COMMON_BRIDGE_L2_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xff,
]);

pub struct L2Hook {
    pub fee_config: FeeConfig,
}

impl Hook for L2Hook {
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), crate::errors::VMError> {
        if !vm.env.is_privileged {
            DefaultHook.prepare_execution(vm)?;

            // Different from L1:

            // Max fee per gas must be sufficient to cover base fee + operator fee
            validate_sufficient_max_fee_per_gas_l2(vm, &self.fee_config.operator_fee_config)?;

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
            vm.current_call_frame.set_code(Code {
                hash: H256::zero(),
                bytecode: vec![Opcode::INVALID.into()].into(),
                jump_targets: Vec::new(),
            })?;
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
            if !ctx_result.is_success() {
                undo_value_transfer(vm)?;
            }

            let gas_refunded: u64 = compute_gas_refunded(vm, ctx_result)?;
            let actual_gas_used = compute_actual_gas_used(vm, gas_refunded, ctx_result.gas_used)?;

            // Different from L1:
            let mut l1_gas = calculate_l1_fee_gas(vm, &self.fee_config.l1_fee_config)?;

            let mut total_gas = actual_gas_used
                .checked_add(l1_gas)
                .ok_or(InternalError::Overflow)?;

            // Check that sender has enough gas to pay the l1 fee
            if total_gas > vm.current_call_frame.gas_limit {
                // Not enough gas to pay l1 fee, force revert

                // Restore VM state to before execution
                vm.substate.revert_backup();
                vm.restore_cache_state()?;

                undo_value_transfer(vm)?;

                ctx_result.result = crate::errors::TxResult::Revert(
                    TxValidationError::InsufficientMaxFeePerGas.into(),
                );
                ctx_result.gas_used = vm.current_call_frame.gas_limit;
                ctx_result.output = Bytes::new();
                // Set l1_gas to use all remaining gas
                l1_gas = vm
                    .current_call_frame
                    .gas_limit
                    .saturating_sub(actual_gas_used);

                total_gas = vm.current_call_frame.gas_limit
            }

            delete_self_destruct_accounts(vm)?;

            // L1 fee is paid to the L1 fee vault
            pay_to_l1_fee_vault(vm, l1_gas, self.fee_config.l1_fee_config)?;

            refund_sender(vm, ctx_result, gas_refunded, total_gas)?;

            // We pay to coinbase after the l1_fee to avoid charging the diff to every transaction
            pay_coinbase_l2(vm, actual_gas_used, &self.fee_config.operator_fee_config)?;

            // Base fee is not burned
            pay_base_fee_vault(vm, actual_gas_used, self.fee_config.base_fee_vault)?;

            // Operator fee is paid to the chain operator
            pay_operator_fee(vm, actual_gas_used, &self.fee_config.operator_fee_config)?;

            ctx_result.gas_used = total_gas;

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

fn validate_sufficient_max_fee_per_gas_l2(
    vm: &mut VM<'_>,
    operator_fee_config: &Option<OperatorFeeConfig>,
) -> Result<(), TxValidationError> {
    let Some(fee_config) = operator_fee_config else {
        // No operator fee configured, this check was done in default hook
        return Ok(());
    };

    let total_fee = vm
        .env
        .base_fee_per_gas
        .checked_add(U256::from(fee_config.operator_fee_per_gas))
        .ok_or(TxValidationError::InsufficientMaxFeePerGas)?;

    if vm.env.tx_max_fee_per_gas.unwrap_or(vm.env.gas_price) < total_fee {
        return Err(TxValidationError::InsufficientMaxFeePerGas);
    }
    Ok(())
}

fn pay_coinbase_l2(
    vm: &mut VM<'_>,
    gas_to_pay: u64,
    operator_fee_config: &Option<OperatorFeeConfig>,
) -> Result<(), crate::errors::VMError> {
    let Some(fee_config) = operator_fee_config else {
        // No operator fee configured, operator fee is not paid
        return pay_coinbase(vm, gas_to_pay);
    };

    let priority_fee_per_gas = vm
        .env
        .gas_price
        .checked_sub(vm.env.base_fee_per_gas)
        .ok_or(InternalError::Underflow)?
        .checked_sub(U256::from(fee_config.operator_fee_per_gas))
        .ok_or(InternalError::Underflow)?;

    let coinbase_fee = U256::from(gas_to_pay)
        .checked_mul(priority_fee_per_gas)
        .ok_or(InternalError::Overflow)?;

    vm.increase_account_balance(vm.env.coinbase, coinbase_fee)?;

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
    gas_to_pay: u64,
    operator_fee_config: &Option<OperatorFeeConfig>,
) -> Result<(), crate::errors::VMError> {
    let Some(fee_config) = operator_fee_config else {
        // No operator fee configured, operator fee is not paid
        return Ok(());
    };

    let operator_fee = U256::from(gas_to_pay)
        .checked_mul(U256::from(fee_config.operator_fee_per_gas))
        .ok_or(InternalError::Overflow)?;

    vm.increase_account_balance(fee_config.operator_fee_vault, operator_fee)?;
    Ok(())
}

fn calculate_l1_fee(
    fee_config: &L1FeeConfig,
    transaction_size: usize,
) -> Result<U256, crate::errors::VMError> {
    let l1_fee_per_blob: U256 = fee_config
        .l1_fee_per_blob_gas
        .checked_mul(GAS_PER_BLOB.into())
        .ok_or(InternalError::Overflow)?
        .into();

    let l1_fee_per_blob_byte = l1_fee_per_blob
        .checked_div(U256::from(SAFE_BYTES_PER_BLOB))
        .ok_or(InternalError::DivisionByZero)?;

    let l1_fee = l1_fee_per_blob_byte
        .checked_mul(U256::from(transaction_size))
        .ok_or(InternalError::Overflow)?;

    Ok(l1_fee)
}

fn calculate_l1_fee_gas(
    vm: &mut VM<'_>,
    l1_fee_config: &Option<L1FeeConfig>,
) -> Result<u64, crate::errors::VMError> {
    let Some(fee_config) = l1_fee_config else {
        // No l1 fee configured, l1 fee gas is zero
        return Ok(0);
    };

    let tx_size = vm.tx.encode_to_vec().len();

    let l1_fee = calculate_l1_fee(fee_config, tx_size)?;
    let mut l1_fee_gas = l1_fee
        .checked_div(vm.env.gas_price)
        .ok_or(InternalError::DivisionByZero)?;

    // Ensure at least 1 gas is charged if there is a non-zero l1 fee
    if l1_fee_gas == U256::zero() && l1_fee > U256::zero() {
        l1_fee_gas = U256::one();
    }

    Ok(l1_fee_gas.try_into().map_err(|_| InternalError::Overflow)?)
}

fn pay_to_l1_fee_vault(
    vm: &mut VM<'_>,
    gas_to_pay: u64,
    l1_fee_config: Option<L1FeeConfig>,
) -> Result<(), crate::errors::VMError> {
    let Some(fee_config) = l1_fee_config else {
        // No l1 fee configured, l1 fee is not paid
        return Ok(());
    };

    let l1_fee = U256::from(gas_to_pay)
        .checked_mul(vm.env.gas_price)
        .ok_or(InternalError::Overflow)?;

    vm.increase_account_balance(fee_config.l1_fee_vault, l1_fee)
        .map_err(|_| TxValidationError::InsufficientAccountFunds)?;

    Ok(())
}
