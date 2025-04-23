use std::cmp::max;

use ethrex_common::{types::Fork, Address, U256};

use crate::{
    constants::{INIT_CODE_MAX_SIZE, TX_BASE_COST, VALID_BLOB_PREFIXES},
    errors::{InternalError, TxValidationError, VMError},
    gas_cost::{self, STANDARD_TOKEN_COST, TOTAL_COST_FLOOR_PER_TOKEN},
    utils::{get_base_fee_per_blob_gas, get_valid_jump_destinations},
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
    fn prepare_execution(&self, vm: &mut crate::vm::VM<'_>) -> Result<(), crate::errors::VMError> {
        vm.increase_account_balance(self.recipient, vm.current_call_frame()?.msg_value)?;

        vm.current_call_frame_mut()?.msg_value = U256::from(0);

        if vm.env.config.fork >= Fork::Prague {
            // check for gas limit is grater or equal than the minimum required
            let intrinsic_gas: u64 = vm.get_intrinsic_gas()?;

            // calldata_cost = tokens_in_calldata * 4
            let calldata_cost: u64 =
                gas_cost::tx_calldata(&vm.current_call_frame()?.calldata, vm.env.config.fork)
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

            if vm.current_call_frame()?.gas_limit < min_gas_limit {
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
            if vm.current_call_frame()?.calldata.len() > INIT_CODE_MAX_SIZE
                && vm.env.config.fork >= Fork::Shanghai
            {
                return Err(VMError::TxValidation(
                    TxValidationError::InitcodeSizeExceeded,
                ));
            }
        }

        // (6) INTRINSIC_GAS_TOO_LOW
        vm.add_intrinsic_gas()?;

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

            vm.eip7702_set_access_code()?;
        }

        if vm.is_create() {
            // Assign bytecode to context and empty calldata
            vm.current_call_frame_mut()?.bytecode =
                std::mem::take(&mut vm.current_call_frame_mut()?.calldata);
            vm.current_call_frame_mut()?.valid_jump_destinations =
                get_valid_jump_destinations(&vm.current_call_frame()?.bytecode).unwrap_or_default();
        }
        Ok(())
    }

    fn finalize_execution(
        &self,
        vm: &mut crate::vm::VM<'_>,
        initial_call_frame: &crate::call_frame::CallFrame,
        report: &mut crate::errors::ExecutionReport,
    ) -> Result<(), crate::errors::VMError> {
        // POST-EXECUTION Changes
        let sender_address = initial_call_frame.msg_sender;

        // 1. Undo value transfer if Tx reverted
        if !report.is_success() {
            // In a create if Tx was reverted the account won't even exist by this point.
            if !vm.is_create() {
                vm.decrease_account_balance(initial_call_frame.to, initial_call_frame.msg_value)?;
            }

            vm.increase_account_balance(sender_address, initial_call_frame.msg_value)?;
        }

        // 2. Return unused gas + gas refunds to the sender.

        // a. Calculate refunded gas
        let gas_used_without_refunds = report.gas_used;

        // [EIP-3529](https://eips.ethereum.org/EIPS/eip-3529)
        // "The max refundable proportion of gas was reduced from one half to one fifth by EIP-3529 by Buterin and Swende [2021] in the London release"
        let refund_quotient = if vm.env.config.fork < Fork::London {
            MAX_REFUND_QUOTIENT_PRE_LONDON
        } else {
            MAX_REFUND_QUOTIENT
        };
        let refunded_gas = report.gas_refunded.min(
            gas_used_without_refunds
                .checked_div(refund_quotient)
                .ok_or(VMError::Internal(InternalError::UndefinedState(-1)))?,
        );

        // b. Calculate actual gas used in the whole transaction. Since Prague there is a base minimum to be consumed.
        let exec_gas_consumed = gas_used_without_refunds
            .checked_sub(refunded_gas)
            .ok_or(VMError::Internal(InternalError::UndefinedState(-2)))?;

        let actual_gas_used = if vm.env.config.fork >= Fork::Prague {
            let minimum_gas_consumed = vm.get_min_gas_used(initial_call_frame)?;
            exec_gas_consumed.max(minimum_gas_consumed)
        } else {
            exec_gas_consumed
        };

        // c. Update gas used and refunded in the Execution Report.
        report.gas_used = actual_gas_used;
        report.gas_refunded = refunded_gas;

        // 3. Pay coinbase fee
        let coinbase_address = vm.env.coinbase;

        let priority_fee_per_gas = vm
            .env
            .gas_price
            .checked_sub(vm.env.base_fee_per_gas)
            .ok_or(VMError::GasPriceIsLowerThanBaseFee)?;
        let coinbase_fee = U256::from(actual_gas_used)
            .checked_mul(priority_fee_per_gas)
            .ok_or(VMError::BalanceOverflow)?;

        vm.increase_account_balance(coinbase_address, coinbase_fee)?;

        // 4. Destruct addresses in vm.estruct set.
        // In Cancun the only addresses destroyed are contracts created in this transaction
        let selfdestruct_set = vm.accrued_substate.selfdestruct_set.clone();
        for address in selfdestruct_set {
            let account_to_remove = vm.get_account_mut(address)?;
            *account_to_remove = Account::default();
        }

        Ok(())
    }
}
