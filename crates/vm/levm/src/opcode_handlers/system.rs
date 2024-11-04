use crate::{
    call_frame::CallFrame,
    constants::{call_opcode, gas_cost, SUCCESS_FOR_RETURN},
    errors::{OpcodeSuccess, ResultReason, VMError},
    vm::{word_to_address, VM},
};
use ethereum_rust_core::{types::TxKind, Address, U256};

// System Operations (10)
// Opcodes: CREATE, CALL, CALLCODE, RETURN, DELEGATECALL, CREATE2, STATICCALL, REVERT, INVALID, SELFDESTRUCT

impl VM {
    // CALL operation
    pub fn op_call(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let gas = current_call_frame.stack.pop()?;
        let code_address = Address::from_low_u64_be(current_call_frame.stack.pop()?.low_u64());
        let value = current_call_frame.stack.pop()?;
        let args_offset = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);
        let args_size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);
        let ret_offset = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);
        let ret_size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);

        if current_call_frame.is_static && !value.is_zero() {
            return Err(VMError::OpcodeNotAllowedInStaticContext);
        }

        let memory_byte_size = (args_offset + args_size).max(ret_offset + ret_size);
        let memory_expansion_cost = current_call_frame.memory.expansion_cost(memory_byte_size)?;

        let positive_value_cost = if !value.is_zero() {
            call_opcode::NON_ZERO_VALUE_COST + call_opcode::BASIC_FALLBACK_FUNCTION_STIPEND
        } else {
            U256::zero()
        };

        let address_access_cost = if !self.cache.is_account_cached(&code_address) {
            self.cache_from_db(&code_address);
            call_opcode::COLD_ADDRESS_ACCESS_COST
        } else {
            call_opcode::WARM_ADDRESS_ACCESS_COST
        };
        let account = self.cache.get_account(code_address).unwrap().clone();

        let value_to_empty_account_cost = if !value.is_zero() && account.is_empty() {
            call_opcode::VALUE_TO_EMPTY_ACCOUNT_COST
        } else {
            U256::zero()
        };

        let gas_cost = memory_expansion_cost
            + address_access_cost
            + positive_value_cost
            + value_to_empty_account_cost;

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
    pub fn op_callcode(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let gas = current_call_frame.stack.pop()?;
        let code_address = Address::from_low_u64_be(current_call_frame.stack.pop()?.low_u64());
        let value = current_call_frame.stack.pop()?;
        let args_offset = current_call_frame.stack.pop()?.try_into().unwrap();
        let args_size = current_call_frame.stack.pop()?.try_into().unwrap();
        let ret_offset = current_call_frame.stack.pop()?.try_into().unwrap();
        let ret_size = current_call_frame.stack.pop()?.try_into().unwrap();

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
        let offset = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);
        let size = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);

        let gas_cost = current_call_frame.memory.expansion_cost(offset + size)?;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        let return_data = current_call_frame.memory.load_range(offset, size).into();
        current_call_frame.returndata = return_data;
        current_call_frame
            .stack
            .push(U256::from(SUCCESS_FOR_RETURN))?;

        Ok(OpcodeSuccess::Result(ResultReason::Return))
    }

    // DELEGATECALL operation
    pub fn op_delegatecall(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let gas = current_call_frame.stack.pop()?;
        let code_address = Address::from_low_u64_be(current_call_frame.stack.pop()?.low_u64());
        let args_offset = current_call_frame.stack.pop()?.try_into().unwrap();
        let args_size = current_call_frame.stack.pop()?.try_into().unwrap();
        let ret_offset = current_call_frame.stack.pop()?.try_into().unwrap();
        let ret_size = current_call_frame.stack.pop()?.try_into().unwrap();

        let msg_sender = current_call_frame.msg_sender;
        let value = current_call_frame.msg_value;
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

    // STATICCALL operation
    pub fn op_staticcall(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let gas = current_call_frame.stack.pop()?;
        let code_address = Address::from_low_u64_be(current_call_frame.stack.pop()?.low_u64());
        let args_offset = current_call_frame.stack.pop()?.try_into().unwrap();
        let args_size = current_call_frame.stack.pop()?.try_into().unwrap();
        let ret_offset = current_call_frame.stack.pop()?.try_into().unwrap();
        let ret_size = current_call_frame.stack.pop()?.try_into().unwrap();

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
            args_offset,
            args_size,
            ret_offset,
            ret_size,
        )
    }

    // CREATE operation
    pub fn op_create(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let value_in_wei_to_send = current_call_frame.stack.pop()?;
        let code_offset_in_memory = current_call_frame.stack.pop()?.try_into().unwrap();
        let code_size_in_memory = current_call_frame.stack.pop()?.try_into().unwrap();

        self.create(
            value_in_wei_to_send,
            code_offset_in_memory,
            code_size_in_memory,
            None,
            current_call_frame,
        )
    }

    // CREATE2 operation
    pub fn op_create2(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let value_in_wei_to_send = current_call_frame.stack.pop()?;
        let code_offset_in_memory = current_call_frame.stack.pop()?.try_into().unwrap();
        let code_size_in_memory = current_call_frame.stack.pop()?.try_into().unwrap();
        let salt = current_call_frame.stack.pop()?;

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

        let gas_cost = current_call_frame.memory.expansion_cost(offset + size)?;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        current_call_frame.returndata = current_call_frame.memory.load_range(offset, size).into();

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

        // Gas costs variables
        let static_gas_cost = gas_cost::SELFDESTRUCT_STATIC;
        let dynamic_gas_cost = gas_cost::SELFDESTRUCT_DYNAMIC;
        let cold_gas_cost = gas_cost::COLD_ADDRESS_ACCESS_COST;
        let mut gas_cost = static_gas_cost;

        // 1. Pop the target address from the stack
        let target_address = word_to_address(current_call_frame.stack.pop()?);

        // 2. Get current account and: Store the balance in a variable, set it's balance to 0
        let mut current_account = self.get_account(&current_call_frame.to);
        let current_account_balance = current_account.info.balance;

        current_account.info.balance = U256::zero();

        // 3 & 4. Get target account and add the balance of the current account to it
        // TODO: If address is cold, there is an additional cost of 2600.
        if !self.cache.is_account_cached(&target_address) {
            gas_cost += cold_gas_cost;
        }

        let mut target_account = self.get_account(&target_address);
        if target_account.is_empty() {
            gas_cost += dynamic_gas_cost;
        }
        target_account.info.balance += current_account_balance;

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
