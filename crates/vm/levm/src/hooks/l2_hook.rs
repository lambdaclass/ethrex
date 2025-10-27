use crate::{
    call_frame::CallFrameBackup,
    constants::POST_OSAKA_GAS_LIMIT_CAP,
    errors::{ContextResult, InternalError, TxValidationError, VMError},
    hooks::{
        DefaultHook,
        default_hook::{
            self, compute_actual_gas_used, compute_gas_refunded, delete_self_destruct_accounts,
            pay_coinbase, refund_sender, set_bytecode_and_code_address, transfer_value,
            undo_value_transfer, validate_gas_allowance, validate_init_code_size,
            validate_min_gas_limit, validate_sender, validate_sufficient_max_fee_per_gas,
        },
        hook::Hook,
    },
    opcodes::Opcode,
    tracing::LevmCallTracer,
    utils::get_account_diffs_in_tx,
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
            account_diff::get_accounts_diff_size,
            fee_config::{FeeConfig, L1FeeConfig, OperatorFeeConfig},
        },
    },
};

pub const COMMON_BRIDGE_L2_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xff,
]);

const LOCK_FEE_SELECTOR: [u8; 4] = [0x89, 0x9c, 0x86, 0xe2];
const PAY_FEE_SELECTOR: [u8; 4] = [0x72, 0x74, 0x6e, 0xaf];

pub struct L2Hook {
    pub fee_config: FeeConfig,
    pub pre_execution_backup: CallFrameBackup,
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

        // Backup the callframe to calculate the tx state diff later
        self.pre_execution_backup = vm.current_call_frame.call_frame_backup.clone();
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
                &mut self.pre_execution_backup,
                vm.env.fee_token.is_some(),
            )?;
        }

        Ok(())
    }
}

fn finalize_non_privileged_execution(
    vm: &mut VM<'_>,
    ctx_result: &mut ContextResult,
    fee_config: &FeeConfig,
    pre_execution_backup: &mut CallFrameBackup,
    use_fee_token: bool,
) -> Result<(), crate::errors::VMError> {
    if !ctx_result.is_success() {
        undo_value_transfer(vm)?;
    }

    let gas_refunded: u64 = compute_gas_refunded(vm, ctx_result)?;
    let actual_gas_used = compute_actual_gas_used(vm, gas_refunded, ctx_result.gas_used)?;

    let mut l1_gas = calculate_l1_fee_gas(
        vm,
        std::mem::take(pre_execution_backup),
        &fee_config.l1_fee_config,
    )?;

    let mut total_gas = actual_gas_used
        .checked_add(l1_gas)
        .ok_or(InternalError::Overflow)?;

    if total_gas > vm.current_call_frame.gas_limit {
        vm.substate.revert_backup();
        vm.restore_cache_state()?;

        undo_value_transfer(vm)?;

        ctx_result.result =
            crate::errors::TxResult::Revert(TxValidationError::InsufficientMaxFeePerGas.into());
        ctx_result.gas_used = vm.current_call_frame.gas_limit;
        ctx_result.output = Bytes::new();

        l1_gas = vm
            .current_call_frame
            .gas_limit
            .saturating_sub(actual_gas_used);
        total_gas = vm.current_call_frame.gas_limit;
    }

    delete_self_destruct_accounts(vm)?;

    if let Some(l1_fee_config) = fee_config.l1_fee_config {
        pay_to_l1_fee_vault(vm, l1_gas, l1_fee_config, use_fee_token)?;
    }

    if use_fee_token {
        refund_sender_fee_token(vm, ctx_result, gas_refunded, total_gas)?;
    } else {
        refund_sender(vm, ctx_result, gas_refunded, total_gas)?;
    }

    pay_coinbase_l2(
        vm,
        actual_gas_used,
        &fee_config.operator_fee_config,
        use_fee_token,
    )?;

    if let Some(base_fee_vault) = fee_config.base_fee_vault {
        pay_base_fee_vault(vm, actual_gas_used, base_fee_vault, use_fee_token)?;
    } else if use_fee_token {
        pay_base_fee_vault(vm, actual_gas_used, Address::zero(), use_fee_token)?;
    }

    if let Some(operator_fee_config) = fee_config.operator_fee_config {
        pay_operator_fee(vm, actual_gas_used, operator_fee_config, use_fee_token)?;
    }

    ctx_result.gas_used = total_gas;

    Ok(())
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
    use_fee_token: bool,
) -> Result<(), crate::errors::VMError> {
    if operator_fee_config.is_none() && !use_fee_token {
        // No operator fee configured, operator fee is not paid
        return pay_coinbase(vm, gas_to_pay);
    }

    let priority_fee_per_gas = compute_priority_fee_per_gas(vm, operator_fee_config)?;

    let coinbase_fee = U256::from(gas_to_pay)
        .checked_mul(priority_fee_per_gas)
        .ok_or(InternalError::Overflow)?;

    if use_fee_token {
        pay_fee_token(vm, vm.env.coinbase, coinbase_fee)?;
    } else {
        vm.increase_account_balance(vm.env.coinbase, coinbase_fee)?;
    }

    Ok(())
}

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

fn compute_operator_fee_amount(
    gas_to_pay: u64,
    fee_config: &OperatorFeeConfig,
) -> Result<U256, InternalError> {
    U256::from(gas_to_pay)
        .checked_mul(U256::from(fee_config.operator_fee_per_gas))
        .ok_or(InternalError::Overflow)
}

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

fn pay_operator_fee(
    vm: &mut VM<'_>,
    gas_to_pay: u64,
    operator_fee_config: OperatorFeeConfig,
    use_fee_token: bool,
) -> Result<(), crate::errors::VMError> {
    let operator_fee = compute_operator_fee_amount(gas_to_pay, &operator_fee_config)?;

    if use_fee_token {
        pay_fee_token(vm, operator_fee_config.operator_fee_vault, operator_fee)?;
    } else {
        vm.increase_account_balance(operator_fee_config.operator_fee_vault, operator_fee)?;
    }
    Ok(())
}

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

fn prepare_execution_fee_token(vm: &mut VM<'_>) -> Result<(), crate::errors::VMError> {
    let sender_address = vm.env.origin;
    let sender_info = vm.db.get_account(sender_address)?.info.clone();

    if vm.env.config.fork >= Fork::Prague {
        validate_min_gas_limit(vm)?;
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
        .gas_price // TODO: here we should ensure that the gas price is the correct ratio from the token erc20 to ETH
        .checked_mul(vm.env.gas_limit.into())
        .ok_or(TxValidationError::GasLimitPriceProductOverflow)?;

    // (2) INSUFFICIENT_MAX_FEE_PER_BLOB_GAS
    // NOT CHECKED: the blob price does not matter, fee token transactions do not support blobs

    // (3) INSUFFICIENT_ACCOUNT_FUNDS
    deduct_caller_fee_token(vm, gaslimit_price_product)?;

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
    // NOT CHECKED: fee token transactions are not type 3

    // Transaction is type 4 if authorization_list is Some
    // NOT CHECKED: fee token transactions are not type 4

    transfer_value(vm)?;

    set_bytecode_and_code_address(vm)?;
    Ok(())
}

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

    // Then, deduct the gas cost in the fee token by locking it in the fee collector
    lock_fee_token(vm, sender_address, gas_limit_price_product)?;

    Ok(())
}

fn encode_fee_token_call(selector: [u8; 4], address: Address, amount: U256) -> Bytes {
    let mut data = Vec::with_capacity(4 + 32 + 32);
    data.extend_from_slice(&selector);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(&address.0);
    data.extend_from_slice(&amount.to_big_endian());
    data.into()
}

fn call_fee_token_contract(
    vm: &mut VM<'_>,
    selector: [u8; 4],
    address: Address,
    amount: U256,
) -> Result<(), VMError> {
    transfer_fee_token(vm, encode_fee_token_call(selector, address, amount))
}

fn lock_fee_token(vm: &mut VM<'_>, payer: Address, amount: U256) -> Result<(), VMError> {
    // lockFee(address payer, uint256 amount) internal onlyFeeCollector
    call_fee_token_contract(vm, LOCK_FEE_SELECTOR, payer, amount)
}

fn pay_fee_token(vm: &mut VM<'_>, receiver: Address, amount: U256) -> Result<(), VMError> {
    // payFee(address receiver, uint256 amount) internal onlyFeeCollector
    call_fee_token_contract(vm, PAY_FEE_SELECTOR, receiver, amount)
}

/// Executes a call to the fee token contract for fee-related operations.
///
/// - This function is only called when locking the fees, refunding unspent gas, and paying the fees to the vaults.
/// - Disable checks as we want to simulate the transaction and get only the updates of the contract storage slots.
/// - This simulation makes a transaction with the calldata provided in `data`, this will be used to call the `payFee` and `lockFee` functions.
///     `lockFee(payer, max_gas_cost)` - locks upfront gas cost from sender
///     `payFee(receiver, amount)` - pays coinbase, vaults, or refunds sender
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

    let mut db_clone = vm.db.clone(); // expensive
    let origin = COMMON_BRIDGE_L2_ADDRESS; // We set the common bridge to restrict access to the fee token contract
    let nonce = db_clone.get_account(origin)?.info.nonce;
    let fee_updates_tx = EIP1559Transaction {
        // we are simulating the transaction
        chain_id: vm.env.chain_id.as_u64(),
        nonce,
        max_priority_fee_per_gas: 100,
        max_fee_per_gas: 100,
        gas_limit: 21000 * 100,
        to: TxKind::Call(fee_token),
        value: U256::zero(),
        data,
        ..Default::default()
    };
    let tx_check_balance = Transaction::EIP1559Transaction(fee_updates_tx);
    let mut env_clone = vm.env.clone();
    // Disable fee checks and update fields
    env_clone.base_fee_per_gas = U256::zero();
    env_clone.block_excess_blob_gas = None;
    env_clone.gas_price = U256::zero();
    env_clone.origin = origin;
    env_clone.fee_token = None;
    env_clone.gas_limit = 21000 * 100;

    let mut new_vm = VM::new(
        env_clone,
        &mut db_clone,
        &tx_check_balance,
        LevmCallTracer::disabled(),
        VMType::L2(Default::default()),
    )?;
    new_vm.hooks = vec![];
    set_bytecode_and_code_address(&mut new_vm)?;
    let execution_result = new_vm.execute()?;
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

fn refund_sender_fee_token(
    vm: &mut VM<'_>,
    ctx_result: &mut ContextResult,
    refunded_gas: u64,
    actual_gas_used: u64,
) -> Result<(), VMError> {
    // c. Update gas used and refunded.
    ctx_result.gas_used = actual_gas_used;
    vm.substate.refunded_gas = refunded_gas;

    // d. Finally, return unspent gas to the sender.
    let gas_to_return = vm
        .env
        .gas_limit
        .checked_sub(actual_gas_used)
        .ok_or(InternalError::Underflow)?;

    let erc20_return_amount = vm
        .env
        .gas_price
        .checked_mul(U256::from(gas_to_return))
        .ok_or(InternalError::Overflow)?;
    let sender_address = vm.env.origin;

    pay_fee_token(vm, sender_address, erc20_return_amount)?;

    Ok(())
}

fn calculate_l1_fee(
    fee_config: &L1FeeConfig,
    account_diffs_size: u64,
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
        .checked_mul(U256::from(account_diffs_size))
        .ok_or(InternalError::Overflow)?;

    Ok(l1_fee)
}

fn calculate_l1_fee_gas(
    vm: &mut VM<'_>,
    pre_execution_backup: CallFrameBackup,
    l1_fee_config: &Option<L1FeeConfig>,
) -> Result<u64, crate::errors::VMError> {
    let Some(fee_config) = l1_fee_config else {
        // No l1 fee configured, l1 fee gas is zero
        return Ok(0);
    };

    let mut execution_backup = vm.current_call_frame.call_frame_backup.clone();
    execution_backup.extend(pre_execution_backup);
    let account_diffs_in_tx = get_account_diffs_in_tx(vm.db, execution_backup)?;
    let account_diffs_size = get_accounts_diff_size(&account_diffs_in_tx)
        .map_err(|e| InternalError::Custom(format!("Failed to get account diffs size: {}", e)))?;

    let l1_fee = calculate_l1_fee(fee_config, account_diffs_size)?;
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
