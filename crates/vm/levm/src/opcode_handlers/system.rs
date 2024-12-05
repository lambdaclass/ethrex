use crate::{
    call_frame::CallFrame,
    constants::WORD_SIZE_IN_BYTES_USIZE,
    errors::{InternalError, OpcodeSuccess, OutOfGasError, ResultReason, VMError},
    gas_cost, memory,
    vm::{word_to_address, VM},
};
use ethrex_core::{types::TxKind, Address, U256};

// System Operations (10)
// Opcodes: CREATE, CALL, CALLCODE, RETURN, DELEGATECALL, CREATE2, STATICCALL, REVERT, INVALID, SELFDESTRUCT

impl VM {
    // CALL operation
    pub fn op_call(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let gas_for_call = current_call_frame.stack.pop()?;
        let callee: Address = word_to_address(current_call_frame.stack.pop()?);
        let value_to_transfer: U256 = current_call_frame.stack.pop()?;

        if current_call_frame.is_static && !value_to_transfer.is_zero() {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }

        let args_start_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let args_size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let return_data_start_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let return_data_size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;

        let new_memory_size_for_args = (args_start_offset
            .checked_add(args_size)
            .ok_or(InternalError::ArithmeticOperationOverflow)?)
        .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
        .ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        let new_memory_size_for_return_data = (return_data_start_offset
            .checked_add(return_data_size)
            .ok_or(InternalError::ArithmeticOperationOverflow)?)
        .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
        .ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);
        let current_memory_size = current_call_frame.memory.len();

        let (account_info, address_was_cold) = self.access_account(callee);

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::call(
                new_memory_size,
                current_memory_size,
                address_was_cold,
                account_info.is_empty(),
                value_to_transfer,
            )?,
        )?;

        let msg_sender = current_call_frame.to; // The new sender will be the current contract.
        let to = callee; // In this case code_address and the sub-context account are the same. Unlike CALLCODE or DELEGATECODE.
        let is_static = current_call_frame.is_static;

        self.generic_call(
            current_call_frame,
            gas_for_call,
            value_to_transfer,
            msg_sender,
            to,
            callee,
            false,
            is_static,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
            true,
        )
    }

    // CALLCODE operation
    // TODO: https://github.com/lambdaclass/ethrex/issues/1086
    pub fn op_callcode(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let gas = current_call_frame.stack.pop()?;
        let code_address = word_to_address(current_call_frame.stack.pop()?);
        let value_to_transfer = current_call_frame.stack.pop()?;
        let args_start_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let args_size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let return_data_start_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let return_data_size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        let new_memory_size_for_args = (args_start_offset
            .checked_add(args_size)
            .ok_or(InternalError::ArithmeticOperationOverflow)?)
        .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
        .ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        let new_memory_size_for_return_data = (return_data_start_offset
            .checked_add(return_data_size)
            .ok_or(InternalError::ArithmeticOperationOverflow)?)
        .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
        .ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);
        let current_memory_size = current_call_frame.memory.len();

        let (_account_info, address_was_cold) = self.access_account(code_address);

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::callcode(
                new_memory_size,
                current_memory_size,
                address_was_cold,
                value_to_transfer,
            )?,
        )?;

        // Sender and recipient are the same in this case. But the code executed is from another account.
        let msg_sender = current_call_frame.to;
        let to = current_call_frame.to;
        let is_static = current_call_frame.is_static;

        self.generic_call(
            current_call_frame,
            gas,
            value_to_transfer,
            msg_sender,
            to,
            code_address,
            false,
            is_static,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
            true,
        )
    }

    // RETURN operation
    pub fn op_return(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;

        let new_memory_size = offset
            .checked_add(size)
            .ok_or(VMError::OutOfGas(OutOfGasError::GasCostOverflow))?;
        self.increase_consumed_gas(
            current_call_frame,
            memory::expansion_cost(new_memory_size, current_call_frame.memory.len())?.into(),
        )?;

        current_call_frame.returndata =
            memory::load_range(&mut current_call_frame.memory, offset, size)?
                .to_vec()
                .into();

        Ok(OpcodeSuccess::Result(ResultReason::Return))
    }

    // DELEGATECALL operation
    // TODO: https://github.com/lambdaclass/ethrex/issues/1086
    pub fn op_delegatecall(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let gas = current_call_frame.stack.pop()?;
        let code_address = word_to_address(current_call_frame.stack.pop()?);
        let args_start_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let args_size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let return_data_start_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let return_data_size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        let msg_sender = current_call_frame.msg_sender;
        let value = current_call_frame.msg_value;
        let to = current_call_frame.to;
        let is_static = current_call_frame.is_static;

        let (_account_info, address_was_cold) = self.access_account(code_address);

        let new_memory_size_for_args = (args_start_offset
            .checked_add(args_size)
            .ok_or(InternalError::ArithmeticOperationOverflow)?)
        .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
        .ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        let new_memory_size_for_return_data = (return_data_start_offset
            .checked_add(return_data_size)
            .ok_or(InternalError::ArithmeticOperationOverflow)?)
        .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
        .ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);
        let current_memory_size = current_call_frame.memory.len();

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::delegatecall(new_memory_size, current_memory_size, address_was_cold)?,
        )?;

        self.generic_call(
            current_call_frame,
            gas,
            value,
            msg_sender,
            to,
            code_address,
            false,
            is_static,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
            false,
        )
    }

    // STATICCALL operation
    // TODO: https://github.com/lambdaclass/ethrex/issues/1086
    pub fn op_staticcall(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let gas = current_call_frame.stack.pop()?;
        let code_address = word_to_address(current_call_frame.stack.pop()?);
        let args_start_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let args_size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let return_data_start_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let return_data_size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        let (_account_info, address_was_cold) = self.access_account(code_address);

        let new_memory_size_for_args = (args_start_offset
            .checked_add(args_size)
            .ok_or(InternalError::ArithmeticOperationOverflow)?)
        .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
        .ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        let new_memory_size_for_return_data = (return_data_start_offset
            .checked_add(return_data_size)
            .ok_or(InternalError::ArithmeticOperationOverflow)?)
        .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
        .ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        let new_memory_size = new_memory_size_for_args.max(new_memory_size_for_return_data);
        let current_memory_size = current_call_frame.memory.len();

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::staticcall(new_memory_size, current_memory_size, address_was_cold)?,
        )?;

        let value = U256::zero();
        let msg_sender = current_call_frame.to; // The new sender will be the current contract.
        let to = code_address; // In this case code_address and the sub-context account are the same. Unlike CALLCODE or DELEGATECODE.

        self.generic_call(
            current_call_frame,
            gas,
            value,
            msg_sender,
            to,
            code_address,
            false,
            true,
            args_start_offset,
            args_size,
            return_data_start_offset,
            return_data_size,
            true,
        )
    }

    // CREATE operation
    // TODO: https://github.com/lambdaclass/ethrex/issues/1086
    pub fn op_create(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let value_in_wei_to_send = current_call_frame.stack.pop()?;
        let code_offset_in_memory: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let code_size_in_memory = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::create(
                code_offset_in_memory
                    .checked_add(code_size_in_memory)
                    .ok_or(VMError::OutOfOffset)?
                    .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
                    .ok_or(VMError::OutOfOffset)?,
                current_call_frame.memory.len(),
                code_size_in_memory,
            )?,
        )?;

        self.create(
            value_in_wei_to_send,
            code_offset_in_memory,
            code_size_in_memory,
            None,
            current_call_frame,
        )
    }

    // CREATE2 operation
    // TODO: https://github.com/lambdaclass/ethrex/issues/1086
    pub fn op_create2(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let value_in_wei_to_send = current_call_frame.stack.pop()?;
        let code_offset_in_memory: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let code_size_in_memory: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let salt = current_call_frame.stack.pop()?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::create_2(
                code_offset_in_memory
                    .checked_add(code_size_in_memory)
                    .ok_or(VMError::OutOfOffset)?
                    .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
                    .ok_or(VMError::OutOfOffset)?,
                current_call_frame.memory.len(),
                code_size_in_memory,
            )?,
        )?;

        self.create(
            value_in_wei_to_send,
            code_offset_in_memory,
            code_size_in_memory,
            Some(salt),
            current_call_frame,
        )
    }

    // REVERT operation
    pub fn op_revert(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        // Description: Gets values from stack, calculates gas cost and sets return data.
        // Returns: VMError RevertOpcode if executed correctly.
        // Notes:
        //      The actual reversion of changes is made in the execute() function.

        let offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        let size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        let new_memory_size = offset
            .checked_add(size)
            .ok_or(VMError::OutOfGas(OutOfGasError::GasCostOverflow))?;
        self.increase_consumed_gas(
            current_call_frame,
            memory::expansion_cost(new_memory_size, current_call_frame.memory.len())?.into(),
        )?;

        current_call_frame.returndata =
            memory::load_range(&mut current_call_frame.memory, offset, size)?
                .to_vec()
                .into();

        Err(VMError::RevertOpcode)
    }

    /// ### INVALID operation
    /// Reverts consuming all gas, no return data.
    pub fn op_invalid(&mut self) -> Result<OpcodeSuccess, VMError> {
        Err(VMError::InvalidOpcode)
    }

    // SELFDESTRUCT operation
    pub fn op_selfdestruct(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        // Sends all ether in the account to the target address
        // Steps:
        // 1. Pop the target address from the stack
        // 2. Get current account and: Store the balance in a variable, set it's balance to 0
        // 3. Get the target account, checking if it is empty and if it is cold. Update gas cost accordingly.
        // 4. Add the balance of the current account to the target account
        // 5. Register account to be destroyed in accrued substate.
        // Notes:
        //      If context is Static, return error.
        //      If executed in the same transaction a contract was created, the current account is registered to be destroyed
        if current_call_frame.is_static {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }

        let target_address = word_to_address(current_call_frame.stack.pop()?);

        let (target_account_info, target_account_is_cold) = self.access_account(target_address);

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::selfdestruct(target_account_is_cold, target_account_info.is_empty())
                .map_err(VMError::OutOfGas)?,
        )?;

        let (current_account_info, _current_account_is_cold) =
            self.access_account(current_call_frame.to);

        self.increase_account_balance(target_address, current_account_info.balance)?;
        self.decrease_account_balance(current_call_frame.to, current_account_info.balance)?;

        if self.tx_kind == TxKind::Create {
            self.accrued_substate
                .selfdestrutct_set
                .insert(current_call_frame.to);
        }

        Ok(OpcodeSuccess::Result(ResultReason::SelfDestruct))
    }
}
