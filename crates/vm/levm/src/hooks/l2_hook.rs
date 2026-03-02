use crate::{
    errors::{ContextResult, InternalError, TxValidationError},
    hooks::{DefaultHook, default_hook, hook::Hook},
    opcodes::Opcode,
    vm::VM,
};

use bytes::Bytes;
use ethrex_common::{
    Address, H160, H256, U256,
    constants::GAS_PER_BLOB,
    types::{
        Code,
        SAFE_BYTES_PER_BLOB,
        fee_config::{FeeConfig, L1FeeConfig, OperatorFeeConfig},
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
        if vm.env.is_privileged {
            return prepare_execution_privileged(vm);
        } else {
            // With custom native gas token, balance field IS the token balance.
            // No fee token simulation needed — standard balance checks work directly.
            DefaultHook.prepare_execution(vm)?;
        }
        // Different from L1:
        // Max fee per gas must be sufficient to cover base fee + operator fee
        validate_sufficient_max_fee_per_gas_l2(vm, &self.fee_config.operator_fee_config)?;
        Ok(())
    }

    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        ctx_result: &mut ContextResult,
    ) -> Result<(), crate::errors::VMError> {
        if vm.env.is_privileged {
            if !ctx_result.is_success() && vm.env.origin != COMMON_BRIDGE_L2_ADDRESS {
                default_hook::undo_value_transfer(vm)?;
            }
            // Even if privileged transactions themselves can't create
            // They can call contracts that use CREATE/CREATE2
            default_hook::delete_self_destruct_accounts(vm)?;
        } else {
            finalize_non_privileged_execution(vm, ctx_result, &self.fee_config)?;
        }

        Ok(())
    }
}

/// Finalizes the execution of a non-privileged L2 transaction.
/// All fees are paid via balance manipulation (no ERC-20 fee token simulation).
/// With custom native gas token, AccountInfo.balance IS the token balance.
fn finalize_non_privileged_execution(
    vm: &mut VM<'_>,
    ctx_result: &mut ContextResult,
    fee_config: &FeeConfig,
) -> Result<(), crate::errors::VMError> {
    if !ctx_result.is_success() {
        default_hook::undo_value_transfer(vm)?;
    }

    // Save pre-refund gas for EIP-7778 block accounting
    let gas_used_pre_refund = ctx_result.gas_used;
    let mut l1_gas = calculate_l1_fee_gas(vm, &fee_config.l1_fee_config)?;

    // EIP-7778: Track pre-refund gas including L1 gas
    let mut total_gas_pre_refund = gas_used_pre_refund
        .checked_add(l1_gas)
        .ok_or(InternalError::Overflow)?;

    let gas_refunded: u64 = default_hook::compute_gas_refunded(vm, ctx_result)?;
    let execution_gas =
        default_hook::compute_actual_gas_used(vm, gas_refunded, gas_used_pre_refund)?;
    let mut actual_gas_used = execution_gas
        .checked_add(l1_gas)
        .ok_or(InternalError::Overflow)?;

    if actual_gas_used > vm.current_call_frame.gas_limit {
        vm.substate.revert_backup();
        vm.restore_cache_state()?;

        default_hook::undo_value_transfer(vm)?;

        ctx_result.result =
            crate::errors::TxResult::Revert(TxValidationError::InsufficientMaxFeePerGas.into());
        ctx_result.gas_used = vm.current_call_frame.gas_limit;
        ctx_result.output = Bytes::new();

        l1_gas = vm
            .current_call_frame
            .gas_limit
            .saturating_sub(execution_gas);
        actual_gas_used = vm.current_call_frame.gas_limit;
        total_gas_pre_refund = vm.current_call_frame.gas_limit;
    }

    default_hook::delete_self_destruct_accounts(vm)?;

    if let Some(l1_fee_config) = fee_config.l1_fee_config {
        pay_to_l1_fee_vault(vm, l1_gas, l1_fee_config)?;
    }

    default_hook::refund_sender(
        vm,
        ctx_result,
        gas_refunded,
        actual_gas_used,
        total_gas_pre_refund,
    )?;

    pay_coinbase_l2(vm, execution_gas, &fee_config.operator_fee_config)?;

    if let Some(base_fee_vault) = fee_config.base_fee_vault {
        pay_base_fee_vault(vm, execution_gas, base_fee_vault)?;
    }

    if let Some(operator_fee_config) = fee_config.operator_fee_config {
        pay_operator_fee(vm, execution_gas, operator_fee_config)?;
    }

    Ok(())
}

fn validate_sufficient_max_fee_per_gas_l2(
    vm: &VM<'_>,
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

/// Pays the coinbase the priority fee per gas for the gas used.
/// If an operator fee config is provided, the priority fee is reduced by the operator fee per gas.
fn pay_coinbase_l2(
    vm: &mut VM<'_>,
    gas_to_pay: u64,
    operator_fee_config: &Option<OperatorFeeConfig>,
) -> Result<(), crate::errors::VMError> {
    if operator_fee_config.is_none() {
        return default_hook::pay_coinbase(vm, gas_to_pay);
    }

    let priority_fee_per_gas = compute_priority_fee_per_gas(vm, operator_fee_config)?;

    let coinbase_fee = U256::from(gas_to_pay)
        .checked_mul(priority_fee_per_gas)
        .ok_or(InternalError::Overflow)?;

    // Per EIP-7928: Coinbase must appear in BAL when there's a user transaction,
    // even if the priority fee is zero. In L2, this function is only called for
    // non-privileged (user) transactions, so no gas_price check is needed.
    if let Some(recorder) = vm.db.bal_recorder.as_mut() {
        recorder.record_touched_address(vm.env.coinbase);
    }

    if !coinbase_fee.is_zero() {
        vm.increase_account_balance(vm.env.coinbase, coinbase_fee)?;
    }

    Ok(())
}

/// Computes the priority fee per gas to be paid to the coinbase.
/// If an operator fee config is provided, the priority fee is reduced by the operator fee per gas.
fn compute_priority_fee_per_gas(
    vm: &VM<'_>,
    operator_fee_config: &Option<OperatorFeeConfig>,
) -> Result<U256, InternalError> {
    let priority_fee = vm
        .env
        .gas_price
        .checked_sub(vm.env.base_fee_per_gas)
        .ok_or(InternalError::Underflow)?;

    if let Some(fee_config) = operator_fee_config {
        priority_fee
            .checked_sub(U256::from(fee_config.operator_fee_per_gas))
            .ok_or(InternalError::Underflow)
    } else {
        Ok(priority_fee)
    }
}

/// Pays the base fee to the base fee vault for the gas used.
/// This is calculated as gas_used * base_fee_per_gas.
fn pay_base_fee_vault(
    vm: &mut VM<'_>,
    gas_to_pay: u64,
    base_fee_vault: Address,
) -> Result<(), crate::errors::VMError> {
    let base_fee = U256::from(gas_to_pay)
        .checked_mul(vm.env.base_fee_per_gas)
        .ok_or(InternalError::Overflow)?;

    vm.increase_account_balance(base_fee_vault, base_fee)?;
    Ok(())
}

/// Pays the operator fee to the operator fee vault for the gas used.
/// This is calculated as gas_used * operator_fee_per_gas.
fn pay_operator_fee(
    vm: &mut VM<'_>,
    gas_to_pay: u64,
    operator_fee_config: OperatorFeeConfig,
) -> Result<(), crate::errors::VMError> {
    let operator_fee = U256::from(gas_to_pay)
        .checked_mul(U256::from(operator_fee_config.operator_fee_per_gas))
        .ok_or(InternalError::Overflow)?;

    vm.increase_account_balance(operator_fee_config.operator_fee_vault, operator_fee)?;
    Ok(())
}

/// Prepares the execution of a privileged transaction.
/// This includes skipping certain checks and validations that are not applicable to privileged transactions.
/// See the comments for details.
fn prepare_execution_privileged(vm: &mut VM<'_>) -> Result<(), crate::errors::VMError> {
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

    // (3) INSUFFICIENT_ACCOUNT_FUNDS
    // NOT CHECKED: privileged transactions do not pay for gas

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

    default_hook::set_bytecode_and_code_address(vm)
}


/// Calculates the L1 fee based on the account diffs size and the L1 fee config.
/// This is done according to the formula:
/// L1 Fee = (L1 Fee per Blob Gas * GAS_PER_BLOB / SAFE_BYTES_PER_BLOB) * account_diffs_size
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

/// Calculates the L1 fee gas based on the account diffs size and the L1 fee config.
/// Returns 0 if no L1 fee config is provided.
fn calculate_l1_fee_gas(
    vm: &VM<'_>,
    l1_fee_config: &Option<L1FeeConfig>,
) -> Result<u64, crate::errors::VMError> {
    let Some(fee_config) = l1_fee_config else {
        // No l1 fee configured, l1 fee gas is zero
        return Ok(0);
    };

    let tx_size = vm.tx.length();

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

/// Pays the L1 fee to the L1 fee vault for the gas used.
/// This is calculated as gas_to_pay * gas_price.
fn pay_to_l1_fee_vault(
    vm: &mut VM<'_>,
    gas_to_pay: u64,
    l1_fee_config: L1FeeConfig,
) -> Result<(), crate::errors::VMError> {
    let l1_fee = U256::from(gas_to_pay)
        .checked_mul(vm.env.gas_price)
        .ok_or(InternalError::Overflow)?;

    vm.increase_account_balance(l1_fee_config.l1_fee_vault, l1_fee)
        .map_err(|_| TxValidationError::InsufficientAccountFunds)?;
    Ok(())
}

