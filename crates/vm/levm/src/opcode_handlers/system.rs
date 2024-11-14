use crate::{
    call_frame::CallFrame,
    constants::SUCCESS_FOR_RETURN,
    errors::{InternalError, OpcodeSuccess, ResultReason, VMError},
    gas_cost,
    vm::{word_to_address, VM},
};
use ethereum_rust_core::{types::TxKind, U256};

// System Operations (10)
// Opcodes: CREATE, CALL, CALLCODE, RETURN, DELEGATECALL, CREATE2, STATICCALL, REVERT, INVALID, SELFDESTRUCT

impl VM {
    // CALL operation
    pub fn op_call(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let gas = current_call_frame.stack.pop()?;
        let code_address = word_to_address(current_call_frame.stack.pop()?);
        let value = current_call_frame.stack.pop()?;
        let args_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let args_size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let ret_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let ret_size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;

        if current_call_frame.is_static && !value.is_zero() {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }

        let is_cached = self.cache.is_account_cached(&code_address);

        if !is_cached {
            self.cache_from_db(&code_address);
        }

        let account_is_empty = self.get_account(&code_address).clone().is_empty();

        let gas_cost = gas_cost::call(
            current_call_frame,
            args_size,
            args_offset,
            ret_size,
            ret_offset,
            value,
            is_cached,
            account_is_empty,
        )
        .map_err(VMError::OutOfGas)?;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        let msg_sender = current_call_frame.to; // The new sender will be the current contract.
        let to = code_address; // In this case code_address and the sub-context account are the same. Unlike CALLCODE or DELEGATECODE.
        let is_static = current_call_frame.is_static;

        self.generic_call(
            current_call_frame,
            gas,
            value,
            msg_sender,
            to,
            code_address,
            false,
            is_static,
            args_offset,
            args_size,
            ret_offset,
            ret_size,
        )
    }

    // CALLCODE operation
    // TODO: https://github.com/lambdaclass/lambda_ethereum_rust/issues/1086
    pub fn op_callcode(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let gas = current_call_frame.stack.pop()?;
        let code_address = word_to_address(current_call_frame.stack.pop()?);
        let value = current_call_frame.stack.pop()?;
        let args_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let args_size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let ret_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let ret_size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        // Gas consumed
        let is_cached = self.cache.is_account_cached(&code_address);

        if !is_cached {
            self.cache_from_db(&code_address);
        };

        let gas_cost = gas_cost::callcode(
            current_call_frame,
            args_size,
            args_offset,
            ret_size,
            ret_offset,
            value,
            is_cached,
        )
        .map_err(VMError::OutOfGas)?;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        // Sender and recipient are the same in this case. But the code executed is from another account.
        let msg_sender = current_call_frame.to;
        let to = current_call_frame.to;
        let is_static = current_call_frame.is_static;

        self.generic_call(
            current_call_frame,
            gas,
            value,
            msg_sender,
            to,
            code_address,
            false,
            is_static,
            args_offset,
            args_size,
            ret_offset,
            ret_size,
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

        let gas_cost =
            current_call_frame
                .memory
                .expansion_cost(offset.checked_add(size).ok_or(VMError::Internal(
                    InternalError::ArithmeticOperationOverflow,
                ))?)?;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        let return_data = current_call_frame.memory.load_range(offset, size)?.into();
        current_call_frame.returndata = return_data;
        current_call_frame
            .stack
            .push(U256::from(SUCCESS_FOR_RETURN))?;

        Ok(OpcodeSuccess::Result(ResultReason::Return))
    }

    // DELEGATECALL operation
    // TODO: https://github.com/lambdaclass/lambda_ethereum_rust/issues/1086
    pub fn op_delegatecall(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let gas = current_call_frame.stack.pop()?;
        let code_address = word_to_address(current_call_frame.stack.pop()?);
        let args_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let args_size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let ret_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let ret_size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        let msg_sender = current_call_frame.msg_sender;
        let value = current_call_frame.msg_value;
        let to = current_call_frame.to;
        let is_static = current_call_frame.is_static;

        // Gas consumed
        let is_cached = self.cache.is_account_cached(&code_address);
        if !is_cached {
            self.cache_from_db(&code_address);
        };

        let gas_cost = gas_cost::delegatecall(
            current_call_frame,
            args_size,
            args_offset,
            ret_size,
            ret_offset,
            is_cached,
        )
        .map_err(VMError::OutOfGas)?;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        self.generic_call(
            current_call_frame,
            gas,
            value,
            msg_sender,
            to,
            code_address,
            false,
            is_static,
            args_offset,
            args_size,
            ret_offset,
            ret_size,
        )
    }

    // STATICCALL operation
    // TODO: https://github.com/lambdaclass/lambda_ethereum_rust/issues/1086
    pub fn op_staticcall(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let gas = current_call_frame.stack.pop()?;
        let code_address = word_to_address(current_call_frame.stack.pop()?);
        let args_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let args_size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let ret_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let ret_size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        let value = U256::zero();
        let msg_sender = current_call_frame.to; // The new sender will be the current contract.
        let to = code_address; // In this case code_address and the sub-context account are the same. Unlike CALLCODE or DELEGATECODE.

        // Gas consumed
        let is_cached = self.cache.is_account_cached(&code_address);

        if !is_cached {
            self.cache_from_db(&code_address);
        };

        let gas_cost = gas_cost::staticcall(
            current_call_frame,
            args_size,
            args_offset,
            ret_size,
            ret_offset,
            is_cached,
        )
        .map_err(VMError::OutOfGas)?;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        self.generic_call(
            current_call_frame,
            gas,
            value,
            msg_sender,
            to,
            code_address,
            false,
            true,
            args_offset,
            args_size,
            ret_offset,
            ret_size,
        )
    }

    // CREATE operation
    // TODO: https://github.com/lambdaclass/lambda_ethereum_rust/issues/1086
    pub fn op_create(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let value_in_wei_to_send = current_call_frame.stack.pop()?;
        let code_offset_in_memory = current_call_frame.stack.pop()?;
        let code_size_in_memory = current_call_frame.stack.pop()?;

        // Gas Cost
        let gas_cost = gas_cost::create(
            current_call_frame,
            code_offset_in_memory,
            code_size_in_memory,
        )
        .map_err(VMError::OutOfGas)?;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        self.create(
            value_in_wei_to_send,
            code_offset_in_memory,
            code_size_in_memory,
            None,
            current_call_frame,
        )
    }

    // CREATE2 operation
    // TODO: https://github.com/lambdaclass/lambda_ethereum_rust/issues/1086
    pub fn op_create2(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let value_in_wei_to_send = current_call_frame.stack.pop()?;
        let code_offset_in_memory = current_call_frame.stack.pop()?;
        let code_size_in_memory = current_call_frame.stack.pop()?;
        let salt = current_call_frame.stack.pop()?;

        // Gas Cost
        let gas_cost = gas_cost::create_2(
            current_call_frame,
            code_offset_in_memory,
            code_size_in_memory,
        )
        .map_err(VMError::OutOfGas)?;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

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

        let offset = current_call_frame.stack.pop()?.as_usize();

        let size = current_call_frame.stack.pop()?.as_usize();

        let gas_cost =
            current_call_frame
                .memory
                .expansion_cost(offset.checked_add(size).ok_or(VMError::Internal(
                    InternalError::ArithmeticOperationOverflow,
                ))?)?;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        current_call_frame.returndata = current_call_frame.memory.load_range(offset, size)?.into();

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

        // 1. Pop the target address from the stack
        let target_address = word_to_address(current_call_frame.stack.pop()?);

        // 2. Get current account and: Store the balance in a variable, set it's balance to 0
        let mut current_account = self.get_account(&current_call_frame.to);
        let current_account_balance = current_account.info.balance;

        current_account.info.balance = U256::zero();

        let is_cached = self.cache.is_account_cached(&target_address);

        // 3 & 4. Get target account and add the balance of the current account to it
        let mut target_account = self.get_account(&target_address);
        let account_is_empty = target_account.is_empty();

        let gas_cost =
            gas_cost::selfdestruct(is_cached, account_is_empty).map_err(VMError::OutOfGas)?;

        target_account.info.balance = target_account
            .info
            .balance
            .checked_add(current_account_balance)
            .ok_or(VMError::BalanceOverflow)?;

        // 5. Register account to be destroyed in accrued substate IF executed in the same transaction a contract was created
        if self.tx_kind == TxKind::Create {
            self.accrued_substate
                .selfdestrutct_set
                .insert(current_call_frame.to);
        }
        // Accounts in SelfDestruct set should be destroyed at the end of the transaction.

        // Update cache after modifying accounts.
        self.cache
            .add_account(&current_call_frame.to, &current_account);
        self.cache.add_account(&target_address, &target_account);

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        Ok(OpcodeSuccess::Result(ResultReason::SelfDestruct))
    }
}
