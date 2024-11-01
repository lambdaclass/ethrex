use crate::{
    call_frame::CallFrame,
    constants::{call_opcode, SUCCESS_FOR_RETURN},
    errors::{OpcodeSuccess, ResultReason, VMError},
    vm::VM,
};
use ethereum_rust_core::{Address, U256};

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
}
