use crate::{
    constants::POST_OSAKA_GAS_LIMIT_CAP,
    db::gen_db::GeneralizedDatabase,
    errors::{ContextResult, ExecutionReport, InternalError, TxValidationError, VMError},
    hooks::{DefaultHook, default_hook, hook::Hook},
    opcodes::Opcode,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};

use bytes::Bytes;
use ethrex_common::{
    Address, H160, H256, U256,
    constants::GAS_PER_BLOB,
    types::{
        Code, EIP1559Transaction, Fork, Transaction, TxKind,
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
pub const FEE_TOKEN_REGISTRY_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfc,
]);
pub const FEE_TOKEN_RATIO_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfb,
]);

// lockFee(address payer, uint256 amount) public onlyBridge
const LOCK_FEE_SELECTOR: [u8; 4] = [0x89, 0x9c, 0x86, 0xe2];
// payFee(address receiver, uint256 amount) public onlyBridge
const PAY_FEE_SELECTOR: [u8; 4] = [0x72, 0x74, 0x6e, 0xaf];
// isFeeToken(address token) external view override returns (bool)
const IS_FEE_TOKEN_SELECTOR: [u8; 4] = [0x16, 0xad, 0x82, 0xd7];
// getFeeTokenRatio(address token) external view returns (uint256)
const FEE_TOKEN_RATIO_SELECTOR: [u8; 4] = [0xc6, 0xab, 0x85, 0xd8];
const SIMULATION_GAS_LIMIT: u64 = 21000 * 100;
const SIMULATION_MAX_FEE: u64 = 100;

pub struct L2Hook {
    pub fee_config: FeeConfig,
}

impl Hook for L2Hook {
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), crate::errors::VMError> {
        if vm.env.is_privileged {
            return prepare_execution_privileged(vm);
        } else if vm.env.fee_token.is_some() {
            prepare_execution_fee_token(vm)?;
        } else {
            DefaultHook.prepare_execution(vm)?;
        }
        // Different from L1:
        // Max fee per gas must be sufficient to cover base fee + operator fee
        validate_sufficient_max_fee_per_gas_l2(vm, &self.fee_config.operator_fee_config)?;
        // Reserve L1 gas from the execution budget so execution can't consume it.
        // If gas_limit < intrinsic_gas + l1_gas, this returns IntrinsicGasTooLow.
        reserve_l1_gas(vm, &self.fee_config.l1_fee_config)?;
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
            finalize_non_privileged_execution(
                vm,
                ctx_result,
                &self.fee_config,
                vm.env.fee_token.is_some(),
            )?;
        }

        Ok(())
    }
}

/// Finalizes the execution of a non-privileged L2 transaction.
/// This will execute the standard checks and requirements as the one defined in the specs or add standard L2 configs applied to general txs.
/// We can set to pay the fees with an ERC20 token instead of ETH.
fn finalize_non_privileged_execution(
    vm: &mut VM<'_>,
    ctx_result: &mut ContextResult,
    fee_config: &FeeConfig,
    use_fee_token: bool,
) -> Result<(), crate::errors::VMError> {
    if !ctx_result.is_success() {
        default_hook::undo_value_transfer(vm)?;
    }

    let l1_gas = calculate_l1_fee_gas(vm, &fee_config.l1_fee_config)?;

    // ctx_result.gas_used includes l1_gas (reserved in prepare_execution).
    // Separate execution gas for refund calculation — l1_gas is not refundable.
    let execution_gas_pre_refund = ctx_result
        .gas_used
        .checked_sub(l1_gas)
        .ok_or(InternalError::Underflow)?;

    // Refund cap based on execution gas only (EIP-3529)
    let gas_refunded: u64 = vm
        .substate
        .refunded_gas
        .min(execution_gas_pre_refund / default_hook::MAX_REFUND_QUOTIENT);
    let execution_gas =
        default_hook::compute_actual_gas_used(vm, gas_refunded, execution_gas_pre_refund)?;

    let actual_gas_used = execution_gas
        .checked_add(l1_gas)
        .ok_or(InternalError::Overflow)?;

    // EIP-7778: pre-refund gas for block accounting
    let total_gas_pre_refund = ctx_result.gas_used;

    default_hook::delete_self_destruct_accounts(vm)?;

    let fee_token_ratio = if let Some(fee_token) = vm.env.fee_token {
        get_fee_token_ratio(vm, fee_token)?
            .try_into()
            .map_err(|_| {
                VMError::Internal(InternalError::Custom(
                    "Failed to convert fee token ratio".to_owned(),
                ))
            })?
    } else {
        1u64
    };

    if let Some(l1_fee_config) = fee_config.l1_fee_config {
        pay_to_l1_fee_vault(
            vm,
            l1_gas.saturating_mul(fee_token_ratio),
            l1_fee_config,
            use_fee_token,
        )?;
    }

    if use_fee_token {
        refund_sender_fee_token(
            vm,
            ctx_result,
            gas_refunded,
            actual_gas_used,
            total_gas_pre_refund,
            fee_token_ratio,
        )?;
    } else {
        default_hook::refund_sender(
            vm,
            ctx_result,
            gas_refunded,
            actual_gas_used,
            total_gas_pre_refund,
        )?;
    }

    pay_coinbase_l2(
        vm,
        execution_gas.saturating_mul(fee_token_ratio),
        &fee_config.operator_fee_config,
        use_fee_token,
    )?;

    // We want to pay the base fee vault if it is set.
    // If not set and it is a fee token transaction we want to burn the fee by sending it
    // to the zero address because it is an ERC20.
    // If not an ERC20 the fees are burned not by a transaction.
    if let Some(base_fee_vault) = fee_config.base_fee_vault {
        pay_base_fee_vault(
            vm,
            execution_gas.saturating_mul(fee_token_ratio),
            base_fee_vault,
            use_fee_token,
        )?;
    } else if use_fee_token {
        pay_base_fee_vault(
            vm,
            execution_gas.saturating_mul(fee_token_ratio),
            Address::zero(),
            use_fee_token,
        )?;
    }

    if let Some(operator_fee_config) = fee_config.operator_fee_config {
        pay_operator_fee(
            vm,
            execution_gas.saturating_mul(fee_token_ratio),
            operator_fee_config,
            use_fee_token,
        )?;
    }

    // Note: ctx_result.gas_used is already correctly set by refund_sender/refund_sender_fee_token
    // based on EIP-7778 (pre-refund for Amsterdam+, post-refund for earlier forks)

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

/// Reserves L1 data availability gas from the execution budget.
///
/// By consuming l1_gas from gas_remaining during prepare_execution,
/// execution physically cannot use the L1 fee portion. This guarantees
/// the L1 fee vault always receives the full l1_gas payment, eliminating
/// the griefing vector where a user sets gas_limit = intrinsic_gas.
///
/// If gas_limit < intrinsic_gas + l1_gas, increase_consumed_gas returns
/// OutOfGas, which we map to IntrinsicGasTooLow to reject the tx upfront.
///
/// On Prague+, also validates gas_limit >= floor + l1_gas (EIP-7623).
/// Finalize computes actual_gas_used = max(execution_gas, floor) + l1_gas,
/// so without this check a tx with heavy calldata could pass validation
/// but underflow in refund_sender.
fn reserve_l1_gas(vm: &mut VM<'_>, l1_fee_config: &Option<L1FeeConfig>) -> Result<(), VMError> {
    let l1_gas = calculate_l1_fee_gas(vm, l1_fee_config)?;

    // On Prague+, the EIP-7623 gas floor can raise execution_gas above
    // intrinsic_gas at finalize time. Since actual_gas_used = execution_gas + l1_gas,
    // we must ensure gas_limit can cover floor + l1_gas.
    if vm.env.config.fork >= Fork::Prague {
        let floor = vm.get_min_gas_used()?;
        let floor_plus_l1 = floor.checked_add(l1_gas).ok_or(InternalError::Overflow)?;
        if vm.env.gas_limit < floor_plus_l1 {
            return Err(TxValidationError::IntrinsicGasTooLow.into());
        }
    }

    vm.current_call_frame
        .increase_consumed_gas(l1_gas)
        .map_err(|_| TxValidationError::IntrinsicGasTooLow)?;
    Ok(())
}

/// Pays the coinbase the priority fee per gas for the gas used.
/// If an operator fee config is provided, the priority fee is reduced by the operator fee per gas.
/// If use_fee_token is true, the fee is paid using the fee token contract.
fn pay_coinbase_l2(
    vm: &mut VM<'_>,
    gas_to_pay: u64,
    operator_fee_config: &Option<OperatorFeeConfig>,
    use_fee_token: bool,
) -> Result<(), crate::errors::VMError> {
    if operator_fee_config.is_none() && !use_fee_token {
        // No operator fee configured, operator fee is not paid
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
        if use_fee_token {
            pay_fee_token(vm, vm.env.coinbase, coinbase_fee)?;
        } else {
            vm.increase_account_balance(vm.env.coinbase, coinbase_fee)?;
        }
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
/// If use_fee_token is true, the fee is paid using the fee token contract.
fn pay_base_fee_vault(
    vm: &mut VM<'_>,
    gas_to_pay: u64,
    base_fee_vault: Address,
    use_fee_token: bool,
) -> Result<(), crate::errors::VMError> {
    let base_fee = U256::from(gas_to_pay)
        .checked_mul(vm.env.base_fee_per_gas)
        .ok_or(InternalError::Overflow)?;

    if use_fee_token {
        pay_fee_token(vm, base_fee_vault, base_fee)?;
    } else {
        vm.increase_account_balance(base_fee_vault, base_fee)?;
    }
    Ok(())
}

/// Pays the operator fee to the operator fee vault for the gas used.
/// This is calculated as gas_used * operator_fee_per_gas.
/// If use_fee_token is true, the fee is paid using the fee token contract.
fn pay_operator_fee(
    vm: &mut VM<'_>,
    gas_to_pay: u64,
    operator_fee_config: OperatorFeeConfig,
    use_fee_token: bool,
) -> Result<(), crate::errors::VMError> {
    let operator_fee = U256::from(gas_to_pay)
        .checked_mul(U256::from(operator_fee_config.operator_fee_per_gas))
        .ok_or(InternalError::Overflow)?;

    if use_fee_token {
        pay_fee_token(vm, operator_fee_config.operator_fee_vault, operator_fee)?;
    } else {
        vm.increase_account_balance(operator_fee_config.operator_fee_vault, operator_fee)?;
    }
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

/// Prepares the execution of a fee token transaction.
/// Similar to default_hook preparation but allows paying fees with ERC20 tokens.
/// Maintains separation between L1 and L2 functionality.
fn prepare_execution_fee_token(vm: &mut VM<'_>) -> Result<(), crate::errors::VMError> {
    let fee_token = vm
        .env
        .fee_token
        .ok_or(VMError::Internal(InternalError::Custom(
            "Fee token address not provided".to_owned(),
        )))?;

    let (execution_result, _) = simulate_common_bridge_call(
        vm,
        FEE_TOKEN_REGISTRY_ADDRESS,
        encode_is_fee_token_call(fee_token),
    )?;

    if !execution_result.is_success() {
        return Err(VMError::TxValidation(
            TxValidationError::InsufficientAccountFunds,
        ));
    }
    // Here we want to check if the token is actually registered as valid.
    // To do this we see if the last byte is 1 or 0.
    // The contract returns a bool that is padded to 32 bytes.
    if execution_result.output.len() != 32
        || execution_result.output.get(31).is_none_or(|&b| b == 0)
    {
        return Err(VMError::TxValidation(
            TxValidationError::InsufficientAccountFunds,
        ));
    }
    let fee_token_ratio = get_fee_token_ratio(vm, fee_token)?;

    let sender_address = vm.env.origin;
    let sender_info = vm.db.get_account(sender_address)?.info.clone();

    if vm.env.config.fork >= Fork::Prague {
        default_hook::validate_min_gas_limit(vm)?;
        if vm.env.config.fork >= Fork::Osaka && vm.tx.gas_limit() > POST_OSAKA_GAS_LIMIT_CAP {
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

    // (2) INSUFFICIENT_MAX_FEE_PER_BLOB_GAS
    // NOT CHECKED: the blob price does not matter, fee token transactions do not support blobs

    // (3) INSUFFICIENT_ACCOUNT_FUNDS
    deduct_caller_fee_token(vm, gaslimit_price_product.saturating_mul(fee_token_ratio))?;

    // (4) INSUFFICIENT_MAX_FEE_PER_GAS
    default_hook::validate_sufficient_max_fee_per_gas(vm)?;

    // (5) INITCODE_SIZE_EXCEEDED
    if vm.is_create()? {
        default_hook::validate_init_code_size(vm)?;
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
    default_hook::validate_sender(sender_address, &code.bytecode)?;

    // (10) GAS_ALLOWANCE_EXCEEDED
    default_hook::validate_gas_allowance(vm)?;

    // Transaction is type 3 if tx_max_fee_per_blob_gas is Some
    // NOT CHECKED: fee token transactions are not type 3

    // Transaction is type 4 if authorization_list is Some
    // NOT CHECKED: fee token transactions are not type 4

    default_hook::transfer_value(vm)?;

    default_hook::set_bytecode_and_code_address(vm)?;
    Ok(())
}

/// Deducts the caller's balance in the fee token for the upfront gas cost.
/// This is calculated as gas_limit * gas_price.
/// This is done through a call to the fee token contract's lockFee function.
pub fn deduct_caller_fee_token(
    vm: &mut VM<'_>,
    gas_limit_price_product: U256,
) -> Result<(), VMError> {
    // Up front cost is the maximum amount of wei that a user is willing to pay for. Gaslimit * gasprice (in ERC20) + value
    let sender_address = vm.env.origin;
    let value = vm.current_call_frame.msg_value;

    // First, try to deduct the value sent
    vm.decrease_account_balance(sender_address, value)
        .map_err(|_| TxValidationError::InsufficientAccountFunds)?;

    // Then, deduct the gas cost in the fee token by locking it in the l2 bridge
    lock_fee_token(vm, sender_address, gas_limit_price_product)?;

    Ok(())
}

/// Helper function to encode the calldata for the fee token contract calls.
/// <function>(address,uint256)
fn encode_fee_token_call(selector: [u8; 4], address: Address, amount: U256) -> Bytes {
    let mut data = Vec::with_capacity(4 + 32 + 32);
    data.extend_from_slice(&selector);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(&address.0);
    data.extend_from_slice(&amount.to_big_endian());
    data.into()
}

fn encode_is_fee_token_call(token: Address) -> Bytes {
    let mut data = Vec::with_capacity(4 + 32);
    data.extend_from_slice(&IS_FEE_TOKEN_SELECTOR);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(&token.0);
    data.into()
}

fn encode_fee_token_ratio_call(token: Address) -> Bytes {
    let mut data = Vec::with_capacity(4 + 32);
    data.extend_from_slice(&FEE_TOKEN_RATIO_SELECTOR);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(&token.0);
    data.into()
}

/// Locks the fee token amount from the payer's balance.
fn lock_fee_token(vm: &mut VM<'_>, payer: Address, amount: U256) -> Result<(), VMError> {
    transfer_fee_token(vm, encode_fee_token_call(LOCK_FEE_SELECTOR, payer, amount))
}

/// Pays the fee token amount to the receiver's balance.
fn pay_fee_token(vm: &mut VM<'_>, receiver: Address, amount: U256) -> Result<(), VMError> {
    transfer_fee_token(
        vm,
        encode_fee_token_call(PAY_FEE_SELECTOR, receiver, amount),
    )
}

/// Executes a call to the fee token contract for fee-related operations.
///
/// - This function is only called when locking the fees, refunding unspent gas, and paying the fees to the vaults.
/// - Disable checks as we want to simulate the transaction and get only the updates of the contract storage slots.
/// - This simulation makes a transaction with the calldata provided in `data`, this will be used to call the `payFee` and `lockFee` functions.
///   `lockFee(payer, max_gas_cost)` - locks upfront gas cost from sender
///   `payFee(receiver, amount)` - pays coinbase, vaults, or refunds sender
/// - Uses `COMMON_BRIDGE_L2_ADDRESS` as origin to restrict access. No user can change this address.
/// - Creates a new VM with cloned database; only fee token storage is synced back.
/// - Uses the same contract address as the one set in the transaction.
fn transfer_fee_token(vm: &mut VM<'_>, data: Bytes) -> Result<(), VMError> {
    let fee_token = vm
        .env
        .fee_token
        .ok_or(VMError::Internal(InternalError::Custom(
            "No fee token address provided, this is a bug".to_owned(),
        )))?;

    let (execution_result, mut db_clone) = simulate_common_bridge_call(vm, fee_token, data)?;

    if !execution_result.is_success() {
        return Err(VMError::TxValidation(
            TxValidationError::InsufficientAccountFunds,
        ));
    }
    let fee_storage = db_clone.get_account(fee_token)?.storage.clone();
    vm.db.get_account_mut(fee_token)?.storage = fee_storage;

    // update the initial state account
    let initial_state_fee_token = db_clone
        .initial_accounts_state
        .get(&fee_token)
        .cloned()
        .ok_or(VMError::Internal(InternalError::Custom(
            "No initial state found for fee token".to_owned(),
        )))?;
    // We have to merge, not insert
    vm.db
        .initial_accounts_state
        .insert(fee_token, initial_state_fee_token);

    Ok(())
}

/// Executes an L2 call as if it originated from the common bridge, returning
/// both the execution report and the mutated database snapshot.
fn simulate_common_bridge_call(
    vm: &VM<'_>,
    to: Address,
    data: Bytes,
) -> Result<(ExecutionReport, GeneralizedDatabase), VMError> {
    let mut db_clone = vm.db.clone(); // expensive but necessary to simulate call
    let origin = COMMON_BRIDGE_L2_ADDRESS; // We set the common bridge to restrict access to the contract
    let nonce = db_clone.get_account(origin)?.info.nonce;
    let simulation_tx = EIP1559Transaction {
        // we are simulating the transaction
        chain_id: vm.env.chain_id.as_u64(),
        nonce,
        max_priority_fee_per_gas: SIMULATION_MAX_FEE,
        max_fee_per_gas: SIMULATION_MAX_FEE,
        gas_limit: SIMULATION_GAS_LIMIT,
        to: TxKind::Call(to),
        value: U256::zero(),
        data,
        ..Default::default()
    };
    let tx = Transaction::EIP1559Transaction(simulation_tx);
    let mut env_clone = vm.env.clone();
    // Disable fee checks and update fields
    env_clone.base_fee_per_gas = U256::zero();
    env_clone.block_excess_blob_gas = None;
    env_clone.gas_price = U256::zero();
    env_clone.origin = origin;
    env_clone.fee_token = None;
    env_clone.gas_limit = SIMULATION_GAS_LIMIT;

    let mut new_vm = VM::new(
        env_clone,
        &mut db_clone,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L2(Default::default()),
    )?;
    new_vm.hooks = vec![];
    default_hook::set_bytecode_and_code_address(&mut new_vm)?;
    let execution_result = new_vm.execute()?;

    Ok((execution_result, db_clone))
}

/// Refunds the sender the unspent gas in fee tokens.
/// Works similarly to refund_sender but uses the fee token contract
/// But we don't want to be mixing L2 logic inside the default hook.
fn refund_sender_fee_token(
    vm: &mut VM<'_>,
    ctx_result: &mut ContextResult,
    refunded_gas: u64,
    gas_spent: u64,
    gas_used_pre_refund: u64,
    fee_token_ratio: u64,
) -> Result<(), VMError> {
    vm.substate.refunded_gas = refunded_gas;

    // EIP-7778: Separate block vs user gas accounting for Amsterdam+
    if vm.env.config.fork >= Fork::Amsterdam {
        // Block accounting uses pre-refund gas
        ctx_result.gas_used = gas_used_pre_refund;
        // User pays post-refund gas
        ctx_result.gas_spent = gas_spent;
    } else {
        // Pre-Amsterdam: both use post-refund value
        ctx_result.gas_used = gas_spent;
        ctx_result.gas_spent = gas_spent;
    }

    // Return unspent gas to the sender.
    let gas_to_return = vm
        .env
        .gas_limit
        .checked_sub(gas_spent)
        .ok_or(InternalError::Underflow)?;

    let erc20_return_amount = vm
        .env
        .gas_price
        .checked_mul(U256::from(gas_to_return))
        .ok_or(InternalError::Overflow)?;
    let sender_address = vm.env.origin;

    pay_fee_token(
        vm,
        sender_address,
        erc20_return_amount.saturating_mul(fee_token_ratio.into()),
    )?;

    Ok(())
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
    use_fee_token: bool,
) -> Result<(), crate::errors::VMError> {
    let l1_fee = U256::from(gas_to_pay)
        .checked_mul(vm.env.gas_price)
        .ok_or(InternalError::Overflow)?;

    if use_fee_token {
        pay_fee_token(vm, l1_fee_config.l1_fee_vault, l1_fee)?;
    } else {
        vm.increase_account_balance(l1_fee_config.l1_fee_vault, l1_fee)
            .map_err(|_| TxValidationError::InsufficientAccountFunds)?;
    }
    Ok(())
}

fn get_fee_token_ratio(vm: &mut VM<'_>, fee_token: H160) -> Result<U256, VMError> {
    let fee_token_ratio = simulate_common_bridge_call(
        vm,
        FEE_TOKEN_RATIO_ADDRESS,
        encode_fee_token_ratio_call(fee_token),
    )?
    .0;
    if !fee_token_ratio.is_success() || fee_token_ratio.output.len() != 32 {
        return Err(VMError::Internal(InternalError::Custom(
            "Failed to get fee token ratio".to_owned(),
        )));
    }
    Ok(U256::from_big_endian(
        fee_token_ratio
            .output
            .get(0..32)
            .ok_or(InternalError::Custom(
                "Failed to parse fee token ratio".to_owned(),
            ))?,
    ))
}
