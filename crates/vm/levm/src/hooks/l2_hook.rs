use std::cmp::max;

use ethrex_common::{types::Fork, Address, U256};

use crate::{
    constants::{INIT_CODE_MAX_SIZE, TX_BASE_COST, VALID_BLOB_PREFIXES},
    db::cache::remove_account,
    errors::{InternalError, TxResult, TxValidationError, VMError},
    gas_cost::{self, STANDARD_TOKEN_COST, TOTAL_COST_FLOOR_PER_TOKEN},
    utils::{
        add_intrinsic_gas, eip7702_set_access_code, get_account, get_account_mut_vm,
        get_base_fee_per_blob_gas, get_intrinsic_gas, get_valid_jump_destinations, has_delegation,
        increase_account_balance,
    },
    Account,
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
        vm: &mut crate::vm::VM,
        initial_call_frame: &mut crate::call_frame::CallFrame,
    ) -> Result<(), crate::errors::VMError> {
        increase_account_balance(
            &mut vm.cache,
            vm.db.clone(),
            self.recipient,
            initial_call_frame.msg_value,
        )?;

        initial_call_frame.msg_value = U256::from(0);

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
        }
        Ok(())
    }

    fn finalize_execution(
        &self,
        vm: &mut crate::vm::VM,
        initial_call_frame: &crate::call_frame::CallFrame,
        report: &mut crate::errors::ExecutionReport,
    ) -> Result<(), crate::errors::VMError> {
        // POST-EXECUTION Changes
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
            } else {
                // We remove the receiver account from the cache, like nothing changed in it's state.
                remove_account(&mut vm.cache, &receiver_address);
            }
        }

        // 2. Return unused gas + gas refunds to the sender.
        let mut consumed_gas = report.gas_used;
        // [EIP-3529](https://eips.ethereum.org/EIPS/eip-3529)
        let quotient = if vm.env.config.fork < Fork::London {
            MAX_REFUND_QUOTIENT_PRE_LONDON
        } else {
            MAX_REFUND_QUOTIENT
        };
        let mut refunded_gas = report.gas_refunded.min(
            consumed_gas
                .checked_div(quotient)
                .ok_or(VMError::Internal(InternalError::UndefinedState(-1)))?,
        );
        // "The max refundable proportion of gas was reduced from one half to one fifth by EIP-3529 by Buterin and Swende [2021] in the London release"
        report.gas_refunded = refunded_gas;

        if vm.env.config.fork >= Fork::Prague {
            let floor_gas_price = vm.get_floor_gas_price(initial_call_frame)?;
            let execution_gas_used = consumed_gas.saturating_sub(refunded_gas);
            if floor_gas_price > execution_gas_used {
                consumed_gas = floor_gas_price;
                refunded_gas = 0;
            }
        }

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
