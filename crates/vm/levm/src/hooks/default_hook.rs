use crate::{
    account::Account,
    call_frame::CallFrame,
    constants::*,
    db::cache::remove_account,
    errors::{ExecutionReport, InternalError, TxResult, TxValidationError, VMError},
    gas_cost::{self, STANDARD_TOKEN_COST, TOTAL_COST_FLOOR_PER_TOKEN},
    hooks::hook::Hook,
    utils::*,
    vm::VM,
};

use ethrex_core::{types::Fork, U256};

use std::cmp::max;

pub struct DefaultHook;

impl Hook for DefaultHook {
    /// ## Description
    /// This method performs validations and returns an error if any of the validations fail.
    /// It also makes pre-execution changes:
    /// - It increases sender nonce
    /// - It substracts up-front-cost from sender balance.
    /// - It adds value to receiver balance.
    /// - It calculates and adds intrinsic gas to the 'gas used' of callframe and environment.
    ///   See 'docs' for more information about validations.
    fn prepare_execution(
        &self,
        vm: &mut VM,
        initial_call_frame: &mut CallFrame,
    ) -> Result<(), VMError> {
        let sender_address = vm.env.origin;
        let sender_account = get_account(&mut vm.cache, vm.db.clone(), sender_address);

        if vm.env.config.fork >= Fork::Prague {
            // check for gas limit is grater or equal than the minimum required
            let intrinsic_gas: u64 = get_intrinsic_gas(
                vm.is_create(),
                vm.env.config.fork,
                &vm.access_list,
                &vm.authorization_list,
                initial_call_frame,
            )?;

            // calldata_cost = tokens_in_calldata * 4
            let calldata_cost: u64 =
                gas_cost::tx_calldata(&initial_call_frame.calldata, vm.env.config.fork)
                    .map_err(VMError::OutOfGas)?;

            // same as calculated in gas_used()
            let tokens_in_calldata: u64 = calldata_cost
                .checked_div(STANDARD_TOKEN_COST)
                .ok_or(VMError::Internal(InternalError::DivisionError))?;

            // floor_cost_by_tokens = TX_BASE_COST + TOTAL_COST_FLOOR_PER_TOKEN * tokens_in_calldata
            let floor_cost_by_tokens = tokens_in_calldata
                .checked_mul(TOTAL_COST_FLOOR_PER_TOKEN)
                .ok_or(VMError::Internal(InternalError::GasOverflow))?
                .checked_add(TX_BASE_COST)
                .ok_or(VMError::Internal(InternalError::GasOverflow))?;

            let min_gas_limit = max(intrinsic_gas, floor_cost_by_tokens);

            if initial_call_frame.gas_limit < min_gas_limit {
                return Err(VMError::TxValidation(TxValidationError::IntrinsicGasTooLow));
            }
        }

        // (1) GASLIMIT_PRICE_PRODUCT_OVERFLOW
        let gaslimit_price_product = vm
            .env
            .gas_price
            .checked_mul(vm.env.gas_limit.into())
            .ok_or(VMError::TxValidation(
                TxValidationError::GasLimitPriceProductOverflow,
            ))?;

        // Up front cost is the maximum amount of wei that a user is willing to pay for. Gaslimit * gasprice + value + blob_gas_cost
        let value = initial_call_frame.msg_value;

        // blob gas cost = max fee per blob gas * blob gas used
        // https://eips.ethereum.org/EIPS/eip-4844
        let max_blob_gas_cost = get_max_blob_gas_price(
            vm.env.tx_blob_hashes.clone(),
            vm.env.tx_max_fee_per_blob_gas,
        )?;

        // For the transaction to be valid the sender account has to have a balance >= gas_price * gas_limit + value if tx is type 0 and 1
        // balance >= max_fee_per_gas * gas_limit + value + blob_gas_cost if tx is type 2 or 3
        let gas_fee_for_valid_tx = vm
            .env
            .tx_max_fee_per_gas
            .unwrap_or(vm.env.gas_price)
            .checked_mul(vm.env.gas_limit.into())
            .ok_or(VMError::TxValidation(
                TxValidationError::GasLimitPriceProductOverflow,
            ))?;

        let balance_for_valid_tx = gas_fee_for_valid_tx
            .checked_add(value)
            .ok_or(VMError::TxValidation(
                TxValidationError::InsufficientAccountFunds,
            ))?
            .checked_add(max_blob_gas_cost)
            .ok_or(VMError::TxValidation(
                TxValidationError::InsufficientAccountFunds,
            ))?;
        if sender_account.info.balance < balance_for_valid_tx {
            return Err(VMError::TxValidation(
                TxValidationError::InsufficientAccountFunds,
            ));
        }

        let blob_gas_cost = get_blob_gas_price(
            vm.env.tx_blob_hashes.clone(),
            vm.env.block_excess_blob_gas,
            &vm.env.config,
        )?;

        // (2) INSUFFICIENT_MAX_FEE_PER_BLOB_GAS
        if let Some(tx_max_fee_per_blob_gas) = vm.env.tx_max_fee_per_blob_gas {
            if tx_max_fee_per_blob_gas
                < get_base_fee_per_blob_gas(vm.env.block_excess_blob_gas, &vm.env.config)?
            {
                return Err(VMError::TxValidation(
                    TxValidationError::InsufficientMaxFeePerBlobGas,
                ));
            }
        }

        // The real cost to deduct is calculated as effective_gas_price * gas_limit + value + blob_gas_cost
        let up_front_cost = gaslimit_price_product
            .checked_add(value)
            .ok_or(VMError::TxValidation(
                TxValidationError::InsufficientAccountFunds,
            ))?
            .checked_add(blob_gas_cost)
            .ok_or(VMError::TxValidation(
                TxValidationError::InsufficientAccountFunds,
            ))?;
        // There is no error specified for overflow in up_front_cost
        // in ef_tests. We went for "InsufficientAccountFunds" simply
        // because if the upfront cost is bigger than U256, then,
        // technically, the sender will not be able to pay it.

        // (3) INSUFFICIENT_ACCOUNT_FUNDS
        decrease_account_balance(&mut vm.cache, vm.db.clone(), sender_address, up_front_cost)
            .map_err(|_| TxValidationError::InsufficientAccountFunds)?;

        // (4) INSUFFICIENT_MAX_FEE_PER_GAS
        if vm.env.tx_max_fee_per_gas.unwrap_or(vm.env.gas_price) < vm.env.base_fee_per_gas {
            return Err(VMError::TxValidation(
                TxValidationError::InsufficientMaxFeePerGas,
            ));
        }

        // (5) INITCODE_SIZE_EXCEEDED
        if vm.is_create() {
            // [EIP-3860] - INITCODE_SIZE_EXCEEDED
            if initial_call_frame.calldata.len() > INIT_CODE_MAX_SIZE
                && vm.env.config.fork >= Fork::Shanghai
            {
                return Err(VMError::TxValidation(
                    TxValidationError::InitcodeSizeExceeded,
                ));
            }
        }

        // (6) INTRINSIC_GAS_TOO_LOW
        add_intrinsic_gas(
            vm.is_create(),
            vm.env.config.fork,
            initial_call_frame,
            &vm.access_list,
            &vm.authorization_list,
        )?;

        // (7) NONCE_IS_MAX
        increment_account_nonce(&mut vm.cache, vm.db.clone(), sender_address)
            .map_err(|_| VMError::TxValidation(TxValidationError::NonceIsMax))?;

        // check for nonce mismatch
        if sender_account.info.nonce != vm.env.tx_nonce {
            return Err(VMError::TxValidation(TxValidationError::NonceMismatch));
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

        // (9) SENDER_NOT_EOA
        if sender_account.has_code() && !has_delegation(&sender_account.info)? {
            return Err(VMError::TxValidation(TxValidationError::SenderNotEOA));
        }

        // (10) GAS_ALLOWANCE_EXCEEDED
        if vm.env.gas_limit > vm.env.block_gas_limit {
            return Err(VMError::TxValidation(
                TxValidationError::GasAllowanceExceeded,
            ));
        }

        // Transaction is type 3 if tx_max_fee_per_blob_gas is Some
        if vm.env.tx_max_fee_per_blob_gas.is_some() {
            // (11) TYPE_3_TX_PRE_FORK
            if vm.env.config.fork < Fork::Cancun {
                return Err(VMError::TxValidation(TxValidationError::Type3TxPreFork));
            }

            let blob_hashes = &vm.env.tx_blob_hashes;

            // (12) TYPE_3_TX_ZERO_BLOBS
            if blob_hashes.is_empty() {
                return Err(VMError::TxValidation(TxValidationError::Type3TxZeroBlobs));
            }

            // (13) TYPE_3_TX_INVALID_BLOB_VERSIONED_HASH
            for blob_hash in blob_hashes {
                let blob_hash = blob_hash.as_bytes();
                if let Some(first_byte) = blob_hash.first() {
                    if !VALID_BLOB_PREFIXES.contains(first_byte) {
                        return Err(VMError::TxValidation(
                            TxValidationError::Type3TxInvalidBlobVersionedHash,
                        ));
                    }
                }
            }

            // (14) TYPE_3_TX_BLOB_COUNT_EXCEEDED
            if blob_hashes.len()
                > vm.env
                    .config
                    .blob_schedule
                    .max
                    .try_into()
                    .map_err(|_| VMError::Internal(InternalError::ConversionError))?
            {
                return Err(VMError::TxValidation(
                    TxValidationError::Type3TxBlobCountExceeded,
                ));
            }

            // (15) TYPE_3_TX_CONTRACT_CREATION
            if vm.is_create() {
                return Err(VMError::TxValidation(
                    TxValidationError::Type3TxContractCreation,
                ));
            }
        }

        // [EIP-7702]: https://eips.ethereum.org/EIPS/eip-7702
        // Transaction is type 4 if authorization_list is Some
        if let Some(auth_list) = &vm.authorization_list {
            // (16) TYPE_4_TX_PRE_FORK
            if vm.env.config.fork < Fork::Prague {
                return Err(VMError::TxValidation(TxValidationError::Type4TxPreFork));
            }

            // (17) TYPE_4_TX_CONTRACT_CREATION
            // From the EIP docs: a null destination is not valid.
            if vm.is_create() {
                return Err(VMError::TxValidation(
                    TxValidationError::Type4TxContractCreation,
                ));
            }

            // (18) TYPE_4_TX_LIST_EMPTY
            // From the EIP docs: The transaction is considered invalid if the length of authorization_list is zero.
            if auth_list.is_empty() {
                return Err(VMError::TxValidation(
                    TxValidationError::Type4TxAuthorizationListIsEmpty,
                ));
            }

            vm.env.refunded_gas = eip7702_set_access_code(
                &mut vm.cache,
                vm.db.clone(),
                vm.env.chain_id,
                &mut vm.accrued_substate,
                // TODO: avoid clone()
                vm.authorization_list.clone(),
                initial_call_frame,
            )?;
        }

        if vm.is_create() {
            // Assign bytecode to context and empty calldata
            initial_call_frame.bytecode = std::mem::take(&mut initial_call_frame.calldata);
            initial_call_frame.valid_jump_destinations =
                get_valid_jump_destinations(&initial_call_frame.bytecode).unwrap_or_default();
        } else {
            // Transfer value to receiver
            // It's here to avoid storing the "to" address in the cache before eip7702_set_access_code() step 7).
            increase_account_balance(
                &mut vm.cache,
                vm.db.clone(),
                initial_call_frame.to,
                initial_call_frame.msg_value,
            )?;
        }
        Ok(())
    }

    /// ## Changes post execution
    /// 1. Undo value transfer if the transaction was reverted
    /// 2. Return unused gas + gas refunds to the sender.
    /// 3. Pay coinbase fee
    /// 4. Destruct addresses in selfdestruct set.
    fn finalize_execution(
        &self,
        vm: &mut VM,
        initial_call_frame: &CallFrame,
        report: &mut ExecutionReport,
    ) -> Result<(), VMError> {
        // POST-EXECUTION Changes
        let sender_address = initial_call_frame.msg_sender;
        let receiver_address = initial_call_frame.to;

        // 1. Undo value transfer if the transaction has reverted
        if let TxResult::Revert(_) = report.result {
            let existing_account = get_account(&mut vm.cache, vm.db.clone(), receiver_address); //TO Account

            if has_delegation(&existing_account.info)? {
                // This is the case where the "to" address and the
                // "signer" address are the same. We are setting the code
                // and sending some balance to the "to"/"signer"
                // address.
                // See https://eips.ethereum.org/EIPS/eip-7702#behavior (last sentence).

                // If transaction execution results in failure (any
                // exceptional condition or code reverting), setting
                // delegation designations is not rolled back.
                decrease_account_balance(
                    &mut vm.cache,
                    vm.db.clone(),
                    receiver_address,
                    initial_call_frame.msg_value,
                )?;
            } else {
                // We remove the receiver account from the cache, like nothing changed in it's state.
                remove_account(&mut vm.cache, &receiver_address);
            }

            increase_account_balance(
                &mut vm.cache,
                vm.db.clone(),
                sender_address,
                initial_call_frame.msg_value,
            )?;
        }

        // 2. Return unused gas + gas refunds to the sender.
        let max_gas = vm.env.gas_limit;
        let consumed_gas = report.gas_used;
        let refunded_gas = report.gas_refunded.min(
            consumed_gas
                .checked_div(5)
                .ok_or(VMError::Internal(InternalError::UndefinedState(-1)))?,
        );
        // "The max refundable proportion of gas was reduced from one half to one fifth by EIP-3529 by Buterin and Swende [2021] in the London release"
        report.gas_refunded = refunded_gas;

        let gas_to_return = max_gas
            .checked_sub(consumed_gas)
            .and_then(|gas| gas.checked_add(refunded_gas))
            .ok_or(VMError::Internal(InternalError::UndefinedState(0)))?;

        let wei_return_amount = vm
            .env
            .gas_price
            .checked_mul(U256::from(gas_to_return))
            .ok_or(VMError::Internal(InternalError::UndefinedState(1)))?;

        increase_account_balance(
            &mut vm.cache,
            vm.db.clone(),
            sender_address,
            wei_return_amount,
        )?;

        // 3. Pay coinbase fee
        let coinbase_address = vm.env.coinbase;

        let gas_to_pay_coinbase = consumed_gas
            .checked_sub(refunded_gas)
            .ok_or(VMError::Internal(InternalError::UndefinedState(2)))?;

        let priority_fee_per_gas = vm
            .env
            .gas_price
            .checked_sub(vm.env.base_fee_per_gas)
            .ok_or(VMError::GasPriceIsLowerThanBaseFee)?;
        let coinbase_fee = U256::from(gas_to_pay_coinbase)
            .checked_mul(priority_fee_per_gas)
            .ok_or(VMError::BalanceOverflow)?;

        if coinbase_fee != U256::zero() {
            increase_account_balance(&mut vm.cache, vm.db.clone(), coinbase_address, coinbase_fee)?;
        };

        // 4. Destruct addresses in vm.estruct set.
        // In Cancun the only addresses destroyed are contracts created in this transaction
        let selfdestruct_set = vm.accrued_substate.selfdestruct_set.clone();
        for address in selfdestruct_set {
            let account_to_remove = get_account_mut_vm(&mut vm.cache, vm.db.clone(), address)?;
            *account_to_remove = Account::default();
        }

        Ok(())
    }
}
