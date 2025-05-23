use crate::{
    constants::*,
    errors::{ExecutionReport, InternalError, OpcodeResult, OutOfGasError, TxResult, VMError},
    gas_cost::CODE_DEPOSIT_COST,
    opcodes::Opcode,
    utils::*,
    vm::VM,
};

use bytes::Bytes;

impl<'a> VM<'a> {
    pub fn handle_precompile_result(
        &mut self,
        precompile_result: Result<Bytes, VMError>,
    ) -> Result<ExecutionReport, VMError> {
        match precompile_result {
            Ok(output) => Ok(ExecutionReport {
                result: TxResult::Success,
                gas_used: self.current_call_frame()?.gas_used,
                gas_refunded: self.substate.refunded_gas,
                output,
                logs: vec![],
            }),
            Err(error) => {
                if error.should_propagate() {
                    return Err(error);
                }

                Ok(ExecutionReport {
                    result: TxResult::Revert(error),
                    gas_used: self.current_call_frame()?.gas_limit,
                    gas_refunded: self.substate.refunded_gas,
                    output: Bytes::new(),
                    logs: vec![],
                })
            }
        }
    }

    pub fn execute_next_instruction(&mut self) -> Result<OpcodeResult, VMError> {
        // Fetches the bytecode for the next instruction
        let instruction_number: u8 = self.current_call_frame()?.fetch_next_instruction_number();

        // Intruction map maps the operation's bytecode to the function that handles said operation
        let instruction_handler = self.instruction_map[instruction_number as usize];

        // Operation handler is called
        instruction_handler(self, instruction_number)
    }

    pub fn handle_opcode_result(&mut self) -> Result<ExecutionReport, VMError> {
        let backup = self
            .substate_backups
            .pop()
            .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))?;
        // On successful create check output validity
        if (self.is_create() && self.current_call_frame()?.depth == 0)
            || self.current_call_frame()?.create_op_called
        {
            let contract_code = std::mem::take(&mut self.current_call_frame_mut()?.output);
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
            } else if contract_code
                .first()
                .is_some_and(|val| val == &INVALID_CONTRACT_PREFIX)
            {
                Err(VMError::InvalidContractPrefix)
            } else if self
                .current_call_frame_mut()?
                .increase_consumed_gas(code_deposit_cost)
                .is_err()
            {
                Err(VMError::OutOfGas(OutOfGasError::MaxGasLimitExceeded))
            } else {
                Ok(self.current_call_frame()?.to)
            };

            match validate_create {
                Ok(new_address) => {
                    // Set bytecode to new account if success
                    self.update_account_bytecode(new_address, contract_code)?;
                }
                Err(error) => {
                    // Revert if error
                    self.current_call_frame_mut()?.gas_used = self.current_call_frame()?.gas_limit;
                    self.restore_state(backup)?;

                    return Ok(ExecutionReport {
                        result: TxResult::Revert(error),
                        gas_used: self.current_call_frame()?.gas_used,
                        gas_refunded: self.substate.refunded_gas,
                        output: std::mem::take(&mut self.current_call_frame_mut()?.output),
                        logs: vec![],
                    });
                }
            }
        }

        Ok(ExecutionReport {
            result: TxResult::Success,
            gas_used: self.current_call_frame()?.gas_used,
            gas_refunded: self.substate.refunded_gas,
            output: std::mem::take(&mut self.current_call_frame_mut()?.output),
            logs: std::mem::take(&mut self.current_call_frame_mut()?.logs),
        })
    }

    pub fn handle_opcode_error(&mut self, error: VMError) -> Result<ExecutionReport, VMError> {
        let backup = self
            .substate_backups
            .pop()
            .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))?;
        if error.should_propagate() {
            return Err(error);
        }

        // Unless error is from Revert opcode, all gas is consumed
        if error != VMError::RevertOpcode {
            let left_gas = self
                .current_call_frame()?
                .gas_limit
                .saturating_sub(self.current_call_frame()?.gas_used);
            self.current_call_frame_mut()?.gas_used =
                self.current_call_frame()?.gas_used.saturating_add(left_gas);
        }

        let refunded = backup.refunded_gas;
        let output = std::mem::take(&mut self.current_call_frame_mut()?.output); // Bytes::new() if error is not RevertOpcode
        let gas_used = self.current_call_frame()?.gas_used;

        self.restore_state(backup)?;

        Ok(ExecutionReport {
            result: TxResult::Revert(error),
            gas_used,
            gas_refunded: refunded,
            output,
            logs: vec![],
        })
    }

    pub fn handle_create_transaction(&mut self) -> Result<Option<ExecutionReport>, VMError> {
        let new_contract_address = self.current_call_frame()?.to;
        let new_account = self.get_account_mut(new_contract_address)?;

        if new_account.has_code_or_nonce() {
            return Ok(Some(ExecutionReport {
                result: TxResult::Revert(VMError::AddressAlreadyOccupied),
                gas_used: self.env.gas_limit,
                gas_refunded: 0,
                logs: vec![],
                output: Bytes::new(),
            }));
        }

        self.increase_account_balance(new_contract_address, self.current_call_frame()?.msg_value)?;

        self.increment_account_nonce(new_contract_address)?;

        Ok(None)
    }
}
