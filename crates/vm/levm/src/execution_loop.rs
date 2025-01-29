use crate::{
    account::{Account, StorageSlot},
    call_frame::CallFrame,
    constants::*,
    db::{
        cache::{self, get_account_mut, remove_account},
        CacheDB, Database,
    },
    environment::Environment,
    errors::{
        InternalError, OpcodeSuccess, OutOfGasError, ResultReason, TransactionReport, TxResult,
        TxValidationError, VMError,
    },
    gas_cost::{self, CODE_DEPOSIT_COST, STANDARD_TOKEN_COST, TOTAL_COST_FLOOR_PER_TOKEN},
    opcodes::Opcode,
    precompiles::{
        execute_precompile, is_precompile, SIZE_PRECOMPILES_CANCUN, SIZE_PRECOMPILES_PRAGUE,
        SIZE_PRECOMPILES_PRE_CANCUN,
    },
    utils::*,
    vm::{AuthorizationList, AuthorizationTuple, Backup, Substate, VM},
    AccountInfo, TransientStorage,
};

use ethrex_core::{U256, U512};

use bytes::Bytes;

impl VM {
    pub fn handle_precompile_result(
        &mut self,
        precompile_result: Result<Bytes, VMError>,
        current_call_frame: &mut CallFrame,
        backup: Backup,
    ) -> Result<TransactionReport, VMError> {
        match precompile_result {
            Ok(output) => {
                self.call_frames.push(current_call_frame.clone());

                return Ok(TransactionReport {
                    result: TxResult::Success,
                    new_state: self.cache.clone(),
                    gas_used: current_call_frame.gas_used,
                    gas_refunded: 0,
                    output,
                    logs: std::mem::take(&mut current_call_frame.logs),
                    created_address: None,
                });
            }
            Err(error) => {
                if error.is_internal() {
                    return Err(error);
                }

                self.call_frames.push(current_call_frame.clone());

                self.restore_state(backup);

                return Ok(TransactionReport {
                    result: TxResult::Revert(error),
                    new_state: CacheDB::default(),
                    gas_used: current_call_frame.gas_limit,
                    gas_refunded: 0,
                    output: Bytes::new(),
                    logs: std::mem::take(&mut current_call_frame.logs),
                    created_address: None,
                });
            }
        }
    }
    pub fn handle_opcode_result(
        &mut self,
        reason: ResultReason,
        current_call_frame: &mut CallFrame,
        backup: Backup,
    ) -> Result<TransactionReport, VMError> {
        self.call_frames.push(current_call_frame.clone());
        // On successful create check output validity
        if (self.is_create() && current_call_frame.depth == 0)
            || current_call_frame.create_op_called
        {
            let contract_code = std::mem::take(&mut current_call_frame.output);
            let code_length = contract_code.len();

            let code_length_u64: u64 = code_length
                .try_into()
                .map_err(|_| VMError::Internal(InternalError::ConversionError))?;

            let code_deposit_cost: u64 =
                code_length_u64
                    .checked_mul(CODE_DEPOSIT_COST)
                    .ok_or(VMError::Internal(
                        InternalError::ArithmeticOperationOverflow,
                    ))?;

            // Revert
            // If the first byte of code is 0xef
            // If the code_length > MAX_CODE_SIZE
            // If current_consumed_gas + code_deposit_cost > gas_limit
            let validate_create = if code_length > MAX_CODE_SIZE {
                Err(VMError::ContractOutputTooBig)
            } else if contract_code.first().unwrap_or(&0) == &INVALID_CONTRACT_PREFIX {
                Err(VMError::InvalidContractPrefix)
            } else if self
                .increase_consumed_gas(current_call_frame, code_deposit_cost)
                .is_err()
            {
                Err(VMError::OutOfGas(OutOfGasError::MaxGasLimitExceeded))
            } else {
                Ok(current_call_frame.to)
            };

            match validate_create {
                Ok(new_address) => {
                    // Set bytecode to new account if success
                    update_account_bytecode(&mut self.cache, &self.db, new_address, contract_code)?;
                }
                Err(error) => {
                    // Revert if error
                    current_call_frame.gas_used = current_call_frame.gas_limit;
                    self.restore_state(backup);

                    return Ok(TransactionReport {
                        result: TxResult::Revert(error),
                        new_state: CacheDB::default(),
                        gas_used: current_call_frame.gas_used,
                        gas_refunded: self.env.refunded_gas,
                        output: std::mem::take(&mut current_call_frame.output),
                        logs: std::mem::take(&mut current_call_frame.logs),
                        created_address: None,
                    });
                }
            }
        }

        return Ok(TransactionReport {
            result: TxResult::Success,
            new_state: CacheDB::default(),
            gas_used: current_call_frame.gas_used,
            gas_refunded: self.env.refunded_gas,
            output: std::mem::take(&mut current_call_frame.output),
            logs: std::mem::take(&mut current_call_frame.logs),
            created_address: None,
        });
    }
}
