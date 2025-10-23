use std::{cell::RefCell, rc::Rc};

use crate::{
    constants::POST_OSAKA_GAS_LIMIT_CAP,
    errors::{ContextResult, InternalError, TxValidationError, VMError},
    hooks::{
        DefaultHook,
        default_hook::{
            self, compute_actual_gas_used, compute_gas_refunded, delete_self_destruct_accounts,
            set_bytecode_and_code_address, transfer_value, undo_value_transfer,
            validate_gas_allowance, validate_init_code_size, validate_min_gas_limit,
            validate_sender, validate_sufficient_max_fee_per_gas,
        },
        empty_hook::EmptyHook,
        hook::Hook,
    },
    opcodes::Opcode,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};

use bytes::Bytes;
use ethrex_common::{
    Address, H160, U256,
    types::{EIP1559Transaction, Fork, Transaction, TxKind, fee_config::FeeConfig},
};

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
            dbg!("PRIVILEGED - PREPARE");
            prepare_execution_privileged(vm)
        } else if vm.env.custom_fee_token.is_some() {
            dbg!("CUSTOM_FEE - PREPARE");
            prepare_execution_custom_fee(vm)
        } else {
            dbg!("DEFAULT - PREPARE");
            DefaultHook.prepare_execution(vm)
        }
    }

    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        ctx_result: &mut ContextResult,
    ) -> Result<(), crate::errors::VMError> {
        if vm.env.is_privileged {
            dbg!("PRIVILEGED - FINALIZE");
            if !ctx_result.is_success() && vm.env.origin != COMMON_BRIDGE_L2_ADDRESS {
                default_hook::undo_value_transfer(vm)?;
            }
            // Even if privileged transactions themselves can't create
            // They can call contracts that use CREATE/CREATE2
            default_hook::delete_self_destruct_accounts(vm)?;
        } else if vm.env.custom_fee_token.is_some() {
            dbg!("CUSTOM_FEE - FINALIZE");
            finalize_execution_custom_fee(vm, ctx_result, self.fee_config.fee_vault)?;
        } else {
            dbg!("DEFAULT - FINALIZE");
            DefaultHook.finalize_execution(vm, ctx_result)?;
            // Different from L1, the base fee is not burned
            return pay_to_fee_vault(vm, ctx_result.gas_used, self.fee_config.fee_vault);
        }

        Ok(())
    }
}

fn pay_to_fee_vault(
    vm: &mut VM<'_>,
    gas_to_pay: u64,
    fee_vault: Option<Address>,
) -> Result<(), crate::errors::VMError> {
    let Some(fee_vault) = fee_vault else {
        // No fee vault configured, base fee is effectively burned
        return Ok(());
    };

    let base_fee = U256::from(gas_to_pay)
        .checked_mul(vm.env.base_fee_per_gas)
        .ok_or(InternalError::Overflow)?;

    vm.increase_account_balance(fee_vault, base_fee)?;
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
        vm.current_call_frame
            .set_code(vec![Opcode::INVALID.into()].into())?;
        return Ok(());
    }

    default_hook::transfer_value(vm)?;

    default_hook::set_bytecode_and_code_address(vm)
}

fn prepare_execution_custom_fee(vm: &mut VM<'_>) -> Result<(), crate::errors::VMError> {
    let sender_address = vm.env.origin;
    let sender_info = vm.db.get_account(sender_address)?.info.clone();
    dbg!(vm.db.get_account(sender_address)?.info.balance);

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

    dbg!(vm.db.get_account(sender_address)?.info.balance);
    // (1) GASLIMIT_PRICE_PRODUCT_OVERFLOW
    let gaslimit_price_product = vm
        .env
        .gas_price // TODO: here we should ensure that the gas price is the correct ratio from the token erc20 to ETH
        .checked_mul(vm.env.gas_limit.into())
        .ok_or(TxValidationError::GasLimitPriceProductOverflow)?;
    dbg!(vm.db.get_account(sender_address)?.info.balance);

    // TODO: validate sender balance for custom fee token

    // (2) INSUFFICIENT_MAX_FEE_PER_BLOB_GAS
    // NOT CHECKED: the blob price does not matter, custom fee transactions do not support blobs

    // (3) INSUFFICIENT_ACCOUNT_FUNDS
    deduct_caller_custom_token(vm, gaslimit_price_product, sender_address)?;
    dbg!(vm.db.get_account(sender_address)?.info.balance);

    // (4) INSUFFICIENT_MAX_FEE_PER_GAS
    validate_sufficient_max_fee_per_gas(vm)?;
    dbg!(vm.db.get_account(sender_address)?.info.balance);

    // (5) INITCODE_SIZE_EXCEEDED
    if vm.is_create()? {
        validate_init_code_size(vm)?;
    }
    dbg!(vm.db.get_account(sender_address)?.info.balance);

    // (6) INTRINSIC_GAS_TOO_LOW
    vm.add_intrinsic_gas()?;
    dbg!(vm.db.get_account(sender_address)?.info.balance);

    // (7) NONCE_IS_MAX
    vm.increment_account_nonce(sender_address)
        .map_err(|_| TxValidationError::NonceIsMax)?;
    dbg!(vm.db.get_account(sender_address)?.info.balance);

    // check for nonce mismatch
    if sender_info.nonce != vm.env.tx_nonce {
        return Err(TxValidationError::NonceMismatch {
            expected: sender_info.nonce,
            actual: vm.env.tx_nonce,
        }
        .into());
    }
    dbg!(vm.db.get_account(sender_address)?.info.balance);

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
    dbg!(vm.db.get_account(sender_address)?.info.balance);

    // (9) SENDER_NOT_EOA
    let code = vm.db.get_code(sender_info.code_hash)?;
    validate_sender(sender_address, code)?;
    dbg!(vm.db.get_account(sender_address)?.info.balance);

    // (10) GAS_ALLOWANCE_EXCEEDED
    validate_gas_allowance(vm)?;
    dbg!(vm.db.get_account(sender_address)?.info.balance);

    // Transaction is type 3 if tx_max_fee_per_blob_gas is Some
    // NOT CHECKED: custom fee transactions are not type 3

    // Transaction is type 4 if authorization_list is Some
    // NOT CHECKED: custom fee transactions are not type 4

    dbg!(sender_address);
    transfer_value(vm)?;
    dbg!(vm.db.get_account(sender_address)?.info.balance);

    set_bytecode_and_code_address(vm)?;
    dbg!(vm.db.get_account(sender_address)?.info.balance);
    Ok(())
}

pub fn deduct_caller_custom_token(
    vm: &mut VM<'_>,
    gas_limit_price_product: U256,
    sender_address: Address,
) -> Result<(), VMError> {
    // Up front cost is the maximum amount of wei that a user is willing to pay for. Gaslimit * gasprice (in ERC20) + value
    let value = vm.current_call_frame.msg_value;

    // First, try to deduct the value sent
    vm.decrease_account_balance(sender_address, value)
        .map_err(|_| TxValidationError::InsufficientAccountFunds)?;

    // Then, deduct the gas cost in the custom fee token
    let sender_address = vm.env.origin;

    /*
    function lockFee(address payer, uint256 amount) internal onlyFeeCollector {
        IERC20(feeToken).transferFrom(payer, address(this), amount);
    }
    */
    // 0x899c86e2
    let lock_fee_selector = vec![0x89, 0x9c, 0x86, 0xe2];
    let mut data = vec![];
    data.extend_from_slice(&lock_fee_selector);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(&sender_address.0);
    data.extend_from_slice(&gas_limit_price_product.to_big_endian());
    transfer_fee_token(vm, data.into())?;

    Ok(())
}

#[allow(clippy::unwrap_used)]
fn transfer_fee_token(vm: &mut VM<'_>, data: Bytes) -> Result<(), VMError> {
    dbg!("a");
    let fee_token = vm.env.custom_fee_token.unwrap();

    dbg!("b");
    let mut db_clone = vm.db.clone(); // expensive
    dbg!("c");
    let sequencer =
        Address::from_slice(&hex::decode("0002bf507275217c9e5ee250bc1b5ca177bb4f74").unwrap());
    dbg!("d");
    let nonce = db_clone.get_account(sequencer)?.info.nonce;
    dbg!("e", sequencer, nonce);
    let tx_check_balance = EIP1559Transaction {
        chain_id: vm.env.chain_id.as_u64(),
        nonce,
        max_priority_fee_per_gas: 9999999,
        max_fee_per_gas: 9999999,
        gas_limit: 999999999,
        to: TxKind::Call(fee_token),
        value: U256::zero(),
        data,
        ..Default::default()
    };
    dbg!("f", &tx_check_balance);
    let tx_check_balance = Transaction::EIP1559Transaction(tx_check_balance);
    dbg!("g", &tx_check_balance);
    let mut env_clone = vm.env.clone();
    dbg!("h");
    // Disable fee checks and update fields
    env_clone.base_fee_per_gas = U256::zero();
    env_clone.block_excess_blob_gas = None;
    env_clone.gas_price = U256::zero();
    env_clone.origin = sequencer;
    env_clone.custom_fee_token = None; // prevent recursion
    env_clone.gas_limit = 999999999;

    let mut new_vm = VM::new(
        env_clone,
        &mut db_clone,
        &tx_check_balance,
        LevmCallTracer::disabled(),
        VMType::L2(Default::default()),
    )?;
    new_vm.hooks = vec![Rc::new(RefCell::new(EmptyHook))];
    dbg!("i");
    set_bytecode_and_code_address(&mut new_vm)?;
    dbg!("j");
    let b = new_vm.execute()?;
    dbg!("k", &b);
    if !b.is_success() {
        return Err(VMError::TxValidation(
            TxValidationError::InsufficientAccountFunds,
        ));
    }
    let fee_storage = db_clone.get_account(fee_token)?.storage.clone();
    dbg!("l");
    dbg!(&vm.db.get_account(fee_token)?.storage, &fee_storage);
    vm.db.get_account_mut(fee_token)?.storage = fee_storage;

    // update the initial state account
    let initial_state_fee_token = db_clone
        .initial_accounts_state
        .get(&fee_token)
        .cloned()
        .unwrap();
    // We have to merge, not insert
    vm.db
        .initial_accounts_state
        .insert(fee_token, initial_state_fee_token);
    dbg!("m");

    Ok(())
}

fn finalize_execution_custom_fee(
    vm: &mut VM<'_>,
    ctx_result: &mut ContextResult,
    fee_vault: Option<Address>,
) -> Result<(), crate::errors::VMError> {
    if !ctx_result.is_success() {
        undo_value_transfer(vm)?;
    }

    let gas_refunded: u64 = compute_gas_refunded(vm, ctx_result)?;
    let actual_gas_used = compute_actual_gas_used(vm, gas_refunded, ctx_result.gas_used)?;

    refund_sender_custom_fee(vm, ctx_result, gas_refunded, actual_gas_used)?;

    pay_coinbase_custom_fee(vm, actual_gas_used)?;

    delete_self_destruct_accounts(vm)?;

    pay_to_fee_vault_custom_fee(vm, ctx_result.gas_used, fee_vault)
}

fn refund_sender_custom_fee(
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

    /*
    function payFee(address receiver, uint256 amount) internal onlyFeeCollector {
        IERC20(feeToken).transferFrom(address(this), receiver, amount);
    }
    */
    // 0x72746eaf
    let pay_fee_selector = vec![0x72, 0x74, 0x6e, 0xaf];
    let mut data = vec![];
    data.extend_from_slice(&pay_fee_selector);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(&sender_address.0);
    data.extend_from_slice(&erc20_return_amount.to_big_endian());
    transfer_fee_token(vm, data.into())?;

    Ok(())
}

fn pay_coinbase_custom_fee(vm: &mut VM<'_>, gas_to_pay: u64) -> Result<(), VMError> {
    let priority_fee_per_gas = vm
        .env
        .gas_price
        .checked_sub(vm.env.base_fee_per_gas)
        .ok_or(InternalError::Underflow)?;

    let coinbase_fee = U256::from(gas_to_pay)
        .checked_mul(priority_fee_per_gas)
        .ok_or(InternalError::Overflow)?;

    /*
    function payFee(address receiver, uint256 amount) internal onlyFeeCollector {
        IERC20(feeToken).transferFrom(address(this), receiver, amount);
    }
    */
    // 0x72746eaf
    let pay_fee_selector = vec![0x72, 0x74, 0x6e, 0xaf];
    let mut data = vec![];
    data.extend_from_slice(&pay_fee_selector);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(&vm.env.coinbase.0);
    data.extend_from_slice(&coinbase_fee.to_big_endian());
    transfer_fee_token(vm, data.into())?;

    Ok(())
}

fn pay_to_fee_vault_custom_fee(
    vm: &mut VM<'_>,
    gas_to_pay: u64,
    fee_vault: Option<Address>,
) -> Result<(), crate::errors::VMError> {
    let Some(fee_vault) = fee_vault else {
        dbg!("=========== BURN ADDRESS ===========");
        let base_fee = U256::from(gas_to_pay)
            .checked_mul(vm.env.base_fee_per_gas)
            .ok_or(InternalError::Overflow)?;
        // 0x72746eaf
        let pay_fee_selector = vec![0x72, 0x74, 0x6e, 0xaf];
        let mut data = vec![];
        data.extend_from_slice(&pay_fee_selector);
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(&[0u8; 20]); // address(0) - burn address
        data.extend_from_slice(&base_fee.to_big_endian());
        transfer_fee_token(vm, data.into())?;
        return Ok(());
    };
    dbg!("=========== FEE VAULT ===========");

    let base_fee = U256::from(gas_to_pay)
        .checked_mul(vm.env.base_fee_per_gas)
        .ok_or(InternalError::Overflow)?;

    /*
    function payFee(address receiver, uint256 amount) internal onlyFeeCollector {
        IERC20(feeToken).transferFrom(address(this), receiver, amount);
    }
    */
    // 0x72746eaf
    let pay_fee_selector = vec![0x72, 0x74, 0x6e, 0xaf];
    let mut data = vec![];
    data.extend_from_slice(&pay_fee_selector);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(&fee_vault.0);
    data.extend_from_slice(&base_fee.to_big_endian());
    transfer_fee_token(vm, data.into())?;
    Ok(())
}
