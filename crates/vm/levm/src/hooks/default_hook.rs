use crate::{
    constants::*,
    errors::{ContextResult, InternalError, TxValidationError, VMError},
    gas_cost::{self, STANDARD_TOKEN_COST, TOTAL_COST_FLOOR_PER_TOKEN},
    hooks::hook::Hook,
    utils::*,
    vm::VM,
};

use ethrex_common::{
    Address, U256,
    types::{Account, Fork},
};

use std::cmp::max;

pub const MAX_REFUND_QUOTIENT: u64 = 5;

pub struct DefaultHook;

impl Hook for DefaultHook {
    /// ## Description
    /// This method performs validations and returns an error if any of these fail.
    /// It also makes pre-execution changes:
    /// - It increases sender nonce
    /// - It substracts up-front-cost from sender balance.
    /// - It adds value to receiver balance.
    /// - It calculates and adds intrinsic gas to the 'gas used' of callframe and environment.
    ///   See 'docs' for more information about validations.
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), VMError> {
        let sender_address = vm.env.origin;
        let (sender_balance, sender_nonce) = {
            let sender_account = vm.db.get_account(sender_address)?;
            (sender_account.info.balance, sender_account.info.nonce)
        };

        if vm.env.config.fork >= Fork::Prague {
            validate_min_gas_limit(vm)?;
        }

        // (1) GASLIMIT_PRICE_PRODUCT_OVERFLOW
        let gaslimit_price_product = vm
            .env
            .gas_price
            .checked_mul(vm.env.gas_limit.into())
            .ok_or(TxValidationError::GasLimitPriceProductOverflow)?;

        validate_sender_balance(vm, sender_balance)?;

        // (2) INSUFFICIENT_MAX_FEE_PER_BLOB_GAS
        if let Some(tx_max_fee_per_blob_gas) = vm.env.tx_max_fee_per_blob_gas {
            validate_max_fee_per_blob_gas(vm, tx_max_fee_per_blob_gas)?;
        }

        // (3) INSUFFICIENT_ACCOUNT_FUNDS
        deduct_caller(vm, gaslimit_price_product, sender_address)?;

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
        if sender_nonce != vm.env.tx_nonce {
            return Err(TxValidationError::NonceMismatch {
                expected: sender_nonce,
                actual: vm.env.tx_nonce,
            }
            .into());
        }

        // (8) PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS
        if let (Some(tx_max_priority_fee), Some(tx_max_fee_per_gas)) = (
            vm.env.tx_max_priority_fee_per_gas,
            vm.env.tx_max_fee_per_gas,
        ) {
            if tx_max_priority_fee > tx_max_fee_per_gas {
                return Err(TxValidationError::PriorityGreaterThanMaxFeePerGas {
                    priority_fee: tx_max_priority_fee,
                    max_fee_per_gas: tx_max_fee_per_gas,
                }
                .into());
            }
        }

        // (9) SENDER_NOT_EOA
        validate_sender(sender_address, vm.db.get_account(sender_address)?)?;

        // (10) GAS_ALLOWANCE_EXCEEDED
        validate_gas_allowance(vm)?;

        // Transaction is type 3 if tx_max_fee_per_blob_gas is Some
        if vm.env.tx_max_fee_per_blob_gas.is_some() {
            validate_4844_tx(vm)?;
        }

        // [EIP-7702]: https://eips.ethereum.org/EIPS/eip-7702
        // Transaction is type 4 if authorization_list is Some
        if vm.tx.authorization_list().is_some() {
            validate_type_4_tx(vm)?;
        }

        transfer_value(vm)?;

        set_bytecode_and_code_address(vm)?;

        Ok(())
    }

    /// ## Changes post execution
    /// 1. Undo value transfer if the transaction was reverted
    /// 2. Return unused gas + gas refunds to the sender.
    /// 3. Pay coinbase fee
    /// 4. Destruct addresses in selfdestruct set.
    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        ctx_result: &mut ContextResult,
    ) -> Result<(), VMError> {
        if !ctx_result.is_success() {
            undo_value_transfer(vm)?;
        }

        let gas_refunded: u64 = compute_gas_refunded(vm, ctx_result)?;
        let actual_gas_used = compute_actual_gas_used(vm, gas_refunded, ctx_result.gas_used)?;
        refund_sender(vm, ctx_result, gas_refunded, actual_gas_used)?;

        pay_coinbase(vm, actual_gas_used)?;

        delete_self_destruct_accounts(vm)?;

        Ok(())
    }
}

pub fn undo_value_transfer(vm: &mut VM<'_>) -> Result<(), VMError> {
    // In a create if Tx was reverted the account won't even exist by this point.
    if !vm.is_create()? {
        vm.decrease_account_balance(
            vm.current_call_frame()?.to,
            vm.current_call_frame()?.msg_value,
        )?;
    }

    vm.increase_account_balance(vm.env.origin, vm.current_call_frame()?.msg_value)?;

    Ok(())
}

pub fn refund_sender(
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

    let wei_return_amount = vm
        .env
        .gas_price
        .checked_mul(U256::from(gas_to_return))
        .ok_or(InternalError::Overflow)?;

    vm.increase_account_balance(vm.env.origin, wei_return_amount)?;

    Ok(())
}

// [EIP-3529](https://eips.ethereum.org/EIPS/eip-3529)
pub fn compute_gas_refunded(vm: &VM<'_>, ctx_result: &ContextResult) -> Result<u64, VMError> {
    Ok(vm
        .substate
        .refunded_gas
        .min(ctx_result.gas_used / MAX_REFUND_QUOTIENT))
}

// Calculate actual gas used in the whole transaction. Since Prague there is a base minimum to be consumed.
pub fn compute_actual_gas_used(
    vm: &mut VM<'_>,
    refunded_gas: u64,
    gas_used_without_refunds: u64,
) -> Result<u64, VMError> {
    let exec_gas_consumed = gas_used_without_refunds
        .checked_sub(refunded_gas)
        .ok_or(InternalError::Underflow)?;

    if vm.env.config.fork >= Fork::Prague {
        Ok(exec_gas_consumed.max(vm.get_min_gas_used()?))
    } else {
        Ok(exec_gas_consumed)
    }
}

pub fn pay_coinbase(vm: &mut VM<'_>, gas_to_pay: u64) -> Result<(), VMError> {
    let priority_fee_per_gas = vm
        .env
        .gas_price
        .checked_sub(vm.env.base_fee_per_gas)
        .ok_or(InternalError::Underflow)?;

    let coinbase_fee = U256::from(gas_to_pay)
        .checked_mul(priority_fee_per_gas)
        .ok_or(InternalError::Overflow)?;

    vm.increase_account_balance(vm.env.coinbase, coinbase_fee)?;

    Ok(())
}

// In Cancun the only addresses destroyed are contracts created in this transaction
pub fn delete_self_destruct_accounts(vm: &mut VM<'_>) -> Result<(), VMError> {
    let selfdestruct_set = vm.substate.selfdestruct_set.clone();
    for address in selfdestruct_set {
        let account_to_remove = vm.get_account_mut(address)?;
        *account_to_remove = Account::default();
        vm.db.destroyed_accounts.insert(address);
    }
    Ok(())
}

pub fn validate_min_gas_limit(vm: &mut VM<'_>) -> Result<(), VMError> {
    // check for gas limit is grater or equal than the minimum required
    let calldata = vm.current_call_frame()?.calldata.clone();
    let intrinsic_gas: u64 = vm.get_intrinsic_gas()?;

    // calldata_cost = tokens_in_calldata * 4
    let calldata_cost: u64 = gas_cost::tx_calldata(&calldata)?;

    // same as calculated in gas_used()
    let tokens_in_calldata: u64 = calldata_cost / STANDARD_TOKEN_COST;

    // floor_cost_by_tokens = TX_BASE_COST + TOTAL_COST_FLOOR_PER_TOKEN * tokens_in_calldata
    let floor_cost_by_tokens = tokens_in_calldata
        .checked_mul(TOTAL_COST_FLOOR_PER_TOKEN)
        .ok_or(InternalError::Overflow)?
        .checked_add(TX_BASE_COST)
        .ok_or(InternalError::Overflow)?;

    let min_gas_limit = max(intrinsic_gas, floor_cost_by_tokens);
    if vm.current_call_frame()?.gas_limit < min_gas_limit {
        return Err(TxValidationError::IntrinsicGasTooLow.into());
    }

    Ok(())
}

pub fn validate_max_fee_per_blob_gas(
    vm: &mut VM<'_>,
    tx_max_fee_per_blob_gas: U256,
) -> Result<(), VMError> {
    let base_fee_per_blob_gas =
        get_base_fee_per_blob_gas(vm.env.block_excess_blob_gas, &vm.env.config)?;
    if tx_max_fee_per_blob_gas < base_fee_per_blob_gas {
        return Err(TxValidationError::InsufficientMaxFeePerBlobGas {
            base_fee_per_blob_gas,
            tx_max_fee_per_blob_gas,
        }
        .into());
    }
    Ok(())
}

pub fn validate_init_code_size(vm: &mut VM<'_>) -> Result<(), VMError> {
    // [EIP-3860] - INITCODE_SIZE_EXCEEDED
    let code_size = vm.current_call_frame()?.calldata.len();
    if code_size > INIT_CODE_MAX_SIZE && vm.env.config.fork >= Fork::Shanghai {
        return Err(TxValidationError::InitcodeSizeExceeded {
            max_size: INIT_CODE_MAX_SIZE,
            actual_size: code_size,
        }
        .into());
    }
    Ok(())
}

pub fn validate_sufficient_max_fee_per_gas(vm: &mut VM<'_>) -> Result<(), TxValidationError> {
    if vm.env.tx_max_fee_per_gas.unwrap_or(vm.env.gas_price) < vm.env.base_fee_per_gas {
        return Err(TxValidationError::InsufficientMaxFeePerGas);
    }
    Ok(())
}

pub fn validate_4844_tx(vm: &mut VM<'_>) -> Result<(), VMError> {
    // (11) TYPE_3_TX_PRE_FORK
    if vm.env.config.fork < Fork::Cancun {
        return Err(TxValidationError::Type3TxPreFork.into());
    }

    let blob_hashes = &vm.env.tx_blob_hashes;

    // (12) TYPE_3_TX_ZERO_BLOBS
    if blob_hashes.is_empty() {
        return Err(TxValidationError::Type3TxZeroBlobs.into());
    }

    // (13) TYPE_3_TX_INVALID_BLOB_VERSIONED_HASH
    for blob_hash in blob_hashes {
        let blob_hash = blob_hash.as_bytes();
        if let Some(first_byte) = blob_hash.first() {
            if !VALID_BLOB_PREFIXES.contains(first_byte) {
                return Err(TxValidationError::Type3TxInvalidBlobVersionedHash.into());
            }
        }
    }

    // (14) TYPE_3_TX_BLOB_COUNT_EXCEEDED
    let max_blob_count = vm
        .env
        .config
        .blob_schedule
        .max
        .try_into()
        .map_err(|_| InternalError::TypeConversion)?;
    let blob_count = blob_hashes.len();
    if blob_count > max_blob_count {
        return Err(TxValidationError::Type3TxBlobCountExceeded {
            max_blob_count,
            actual_blob_count: blob_count,
        }
        .into());
    }

    // (15) TYPE_3_TX_CONTRACT_CREATION
    // NOTE: This will never happen, since the EIP-4844 tx (type 3) does not have a TxKind field
    // only supports an Address which must be non-empty.
    // If a type 3 tx has the field `to` as null (signaling create), it will raise an exception on RLP decoding,
    // it won't reach this point.
    // For more information, please check the following thread:
    // - https://github.com/lambdaclass/ethrex/pull/2425/files/819825516dc633275df56b2886b921061c4d7681#r2035611105
    if vm.is_create()? {
        return Err(TxValidationError::Type3TxContractCreation.into());
    }

    Ok(())
}

pub fn validate_type_4_tx(vm: &mut VM<'_>) -> Result<(), VMError> {
    let Some(auth_list) = vm.tx.authorization_list() else {
        // vm.authorization_list should be Some at this point.
        return Err(InternalError::Custom("Auth list not found".to_string()).into());
    };

    // (16) TYPE_4_TX_PRE_FORK
    if vm.env.config.fork < Fork::Prague {
        return Err(TxValidationError::Type4TxPreFork.into());
    }

    // (17) TYPE_4_TX_CONTRACT_CREATION
    // From the EIP docs: a null destination is not valid.
    // NOTE: This will never happen, since the EIP-7702 tx (type 4) does not have a TxKind field
    // only supports an Address which must be non-empty.
    // If a type 4 tx has the field `to` as null (signaling create), it will raise an exception on RLP decoding,
    // it won't reach this point.
    // For more information, please check the following thread:
    // - https://github.com/lambdaclass/ethrex/pull/2425/files/819825516dc633275df56b2886b921061c4d7681#r2035611105
    if vm.is_create()? {
        return Err(TxValidationError::Type4TxContractCreation.into());
    }

    // (18) TYPE_4_TX_LIST_EMPTY
    // From the EIP docs: The transaction is considered invalid if the length of authorization_list is zero.
    if auth_list.is_empty() {
        return Err(TxValidationError::Type4TxAuthorizationListIsEmpty.into());
    }

    vm.eip7702_set_access_code()
}

pub fn validate_sender(sender_address: Address, sender_account: &Account) -> Result<(), VMError> {
    if sender_account.has_code() && !has_delegation(sender_account)? {
        return Err(TxValidationError::SenderNotEOA(sender_address).into());
    }
    Ok(())
}

pub fn validate_gas_allowance(vm: &mut VM<'_>) -> Result<(), TxValidationError> {
    if vm.env.gas_limit > vm.env.block_gas_limit {
        return Err(TxValidationError::GasAllowanceExceeded {
            block_gas_limit: vm.env.block_gas_limit,
            tx_gas_limit: vm.env.gas_limit,
        });
    }
    Ok(())
}

pub fn validate_sender_balance(vm: &mut VM<'_>, sender_balance: U256) -> Result<(), VMError> {
    // Up front cost is the maximum amount of wei that a user is willing to pay for. Gaslimit * gasprice + value + blob_gas_cost
    let value = vm.current_call_frame()?.msg_value;

    // blob gas cost = max fee per blob gas * blob gas used
    // https://eips.ethereum.org/EIPS/eip-4844
    let max_blob_gas_cost =
        get_max_blob_gas_price(&vm.env.tx_blob_hashes, vm.env.tx_max_fee_per_blob_gas)?;

    // For the transaction to be valid the sender account has to have a balance >= gas_price * gas_limit + value if tx is type 0 and 1
    // balance >= max_fee_per_gas * gas_limit + value + blob_gas_cost if tx is type 2 or 3
    let gas_fee_for_valid_tx = vm
        .env
        .tx_max_fee_per_gas
        .unwrap_or(vm.env.gas_price)
        .checked_mul(vm.env.gas_limit.into())
        .ok_or(TxValidationError::GasLimitPriceProductOverflow)?;

    let balance_for_valid_tx = gas_fee_for_valid_tx
        .checked_add(value)
        .ok_or(TxValidationError::InsufficientAccountFunds)?
        .checked_add(max_blob_gas_cost)
        .ok_or(TxValidationError::InsufficientAccountFunds)?;

    if sender_balance < balance_for_valid_tx {
        return Err(TxValidationError::InsufficientAccountFunds.into());
    }

    Ok(())
}

pub fn deduct_caller(
    vm: &mut VM<'_>,
    gas_limit_price_product: U256,
    sender_address: Address,
) -> Result<(), VMError> {
    // Up front cost is the maximum amount of wei that a user is willing to pay for. Gaslimit * gasprice + value + blob_gas_cost
    let value = vm.current_call_frame()?.msg_value;

    let blob_gas_cost = get_blob_gas_price(
        &vm.env.tx_blob_hashes,
        vm.env.block_excess_blob_gas,
        &vm.env.config,
    )?;

    // The real cost to deduct is calculated as effective_gas_price * gas_limit + value + blob_gas_cost
    let up_front_cost = gas_limit_price_product
        .checked_add(value)
        .ok_or(TxValidationError::InsufficientAccountFunds)?
        .checked_add(blob_gas_cost)
        .ok_or(TxValidationError::InsufficientAccountFunds)?;
    // There is no error specified for overflow in up_front_cost
    // in ef_tests. We went for "InsufficientAccountFunds" simply
    // because if the upfront cost is bigger than U256, then,
    // technically, the sender will not be able to pay it.

    vm.decrease_account_balance(sender_address, up_front_cost)
        .map_err(|_| TxValidationError::InsufficientAccountFunds)?;

    Ok(())
}

/// Transfer msg_value to transaction recipient
pub fn transfer_value(vm: &mut VM<'_>) -> Result<(), VMError> {
    if !vm.is_create()? {
        vm.increase_account_balance(
            vm.current_call_frame()?.to,
            vm.current_call_frame()?.msg_value,
        )?;
    }
    Ok(())
}

/// Sets bytecode and code_address to CallFrame
pub fn set_bytecode_and_code_address(vm: &mut VM<'_>) -> Result<(), VMError> {
    // Get bytecode and code_address for assigning those values to the callframe.
    let (bytecode, code_address) = if vm.is_create()? {
        // Here bytecode is the calldata and the code_address is just the created contract address.
        let calldata = std::mem::take(&mut vm.current_call_frame_mut()?.calldata);
        (calldata, vm.current_call_frame()?.to)
    } else {
        // Here bytecode and code_address could be either from the account or from the delegated account.
        let to = vm.current_call_frame()?.to;
        let (_is_delegation, _eip7702_gas_consumed, code_address, bytecode) =
            eip7702_get_code(vm.db, &mut vm.substate, to)?;

        (bytecode, code_address)
    };

    // Assign code and code_address to callframe
    vm.current_call_frame_mut()?.code_address = code_address;
    vm.current_call_frame_mut()?.set_code(bytecode)?;

    Ok(())
}
