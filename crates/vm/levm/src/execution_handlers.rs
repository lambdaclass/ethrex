use crate::{
    constants::*,
    errors::{ContextResult, ExceptionalHalt, InternalError, TxResult, VMError},
    gas_cost::CODE_DEPOSIT_COST,
    vm::VM,
};

use bytes::Bytes;

impl<'a> VM<'a> {
    pub fn handle_precompile_result(
        precompile_result: Result<Bytes, VMError>,
        gas_limit: u64,
        gas_remaining: u64,
    ) -> Result<ContextResult, VMError> {
        match precompile_result {
            Ok(output) => Ok(ContextResult {
                result: TxResult::Success,
                gas_used: gas_limit
                    .checked_sub(gas_remaining)
                    .ok_or(InternalError::Underflow)?,
                output,
            }),
            Err(error) => {
                if error.should_propagate() {
                    return Err(error);
                }

                Ok(ContextResult {
                    result: TxResult::Revert(error),
                    gas_used: gas_limit,
                    output: Bytes::new(),
                })
            }
        }
    }

    #[cold] // used in the hot path loop, called only really once.
    pub fn handle_opcode_result(&mut self) -> Result<ContextResult, VMError> {
        // On successful create check output validity
        if self.is_create()? {
            let validate_create = self.validate_contract_creation();

            if let Err(error) = validate_create {
                if error.should_propagate() {
                    return Err(error);
                }

                // Consume all gas because error was exceptional.
                let callframe = &mut self.current_call_frame;
                callframe.gas_remaining = 0;

                return Ok(ContextResult {
                    result: TxResult::Revert(error),
                    gas_used: callframe
                        .gas_limit
                        .checked_sub(callframe.gas_remaining as u64)
                        .ok_or(InternalError::Underflow)?,
                    output: Bytes::new(),
                });
            }

            // Set bytecode to the newly created contract.
            let contract_address = self.current_call_frame.to;
            let code = self.current_call_frame.output.clone();
            self.update_account_bytecode(contract_address, code)?;
        }

        Ok(ContextResult {
            result: TxResult::Success,
            gas_used: {
                let callframe = &mut self.current_call_frame;
                callframe
                    .gas_limit
                    .checked_sub(callframe.gas_remaining as u64)
                    .ok_or(InternalError::Underflow)?
            },
            output: std::mem::take(&mut self.current_call_frame.output),
        })
    }

    #[cold] // used in the hot path loop, called only really once.
    pub fn handle_opcode_error(&mut self, error: VMError) -> Result<ContextResult, VMError> {
        if error.should_propagate() {
            return Err(error);
        }

        let callframe = &mut self.current_call_frame;

        // Unless error is caused by Revert Opcode, consume all gas left.
        if !error.is_revert_opcode() {
            callframe.gas_remaining = 0;
        }

        Ok(ContextResult {
            result: TxResult::Revert(error),
            gas_used: callframe
                .gas_limit
                .checked_sub(callframe.gas_remaining as u64)
                .ok_or(InternalError::Underflow)?,
            output: std::mem::take(&mut callframe.output),
        })
    }

    /// Handles external create transaction.
    pub fn handle_create_transaction(&mut self) -> Result<Option<ContextResult>, VMError> {
        let new_contract_address = self.current_call_frame.to;
        let new_account = self.get_account_mut(new_contract_address)?;

        if new_account.has_code_or_nonce() {
            return Ok(Some(ContextResult {
                result: TxResult::Revert(ExceptionalHalt::AddressAlreadyOccupied.into()),
                gas_used: self.env.gas_limit,
                output: Bytes::new(),
            }));
        }

        self.increase_account_balance(new_contract_address, self.current_call_frame.msg_value)?;

        self.increment_account_nonce(new_contract_address)?;

        Ok(None)
    }

    /// Validates that the contract creation was successful, otherwise it returns an ExceptionalHalt.
    fn validate_contract_creation(&mut self) -> Result<(), VMError> {
        let callframe = &mut self.current_call_frame;
        let code = &callframe.output;

        let code_length: u64 = code
            .len()
            .try_into()
            .map_err(|_| InternalError::TypeConversion)?;

        let code_deposit_cost: u64 = code_length
            .checked_mul(CODE_DEPOSIT_COST)
            .ok_or(InternalError::Overflow)?;

        // Revert Scenarios
        // 1. If the first byte of code is 0xEF
        if code.first().is_some_and(|v| v == &EOF_PREFIX) {
            return Err(ExceptionalHalt::InvalidContractPrefix.into());
        }

        // 2. If the code_length > MAX_CODE_SIZE
        if code_length > MAX_CODE_SIZE {
            return Err(ExceptionalHalt::ContractOutputTooBig.into());
        }

        // 3. current_consumed_gas + code_deposit_cost > gas_limit
        callframe.increase_consumed_gas(code_deposit_cost)?;

        Ok(())
    }
}
