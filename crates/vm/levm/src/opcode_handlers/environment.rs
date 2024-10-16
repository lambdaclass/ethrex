use crate::constants::WORD_SIZE;

use super::*;

// Environmental Information (16)
// Opcodes: ADDRESS, BALANCE, ORIGIN, CALLER, CALLVALUE, CALLDATALOAD, CALLDATASIZE, CALLDATACOPY, CODESIZE, CODECOPY, GASPRICE, EXTCODESIZE, EXTCODECOPY, RETURNDATASIZE, RETURNDATACOPY, EXTCODEHASH

impl VM {
    // CALLDATALOAD operation
    pub fn op_calldataload(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::CALLDATALOAD > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        let offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);
        let value = U256::from_big_endian(&current_call_frame.calldata.slice(offset..offset + 32));
        current_call_frame.stack.push(value)?;
        self.env.consumed_gas += gas_cost::CALLDATALOAD;

        Ok(OpcodeSuccess::Continue)
    }

    // CALLDATASIZE operation
    pub fn op_calldatasize(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::CALLDATASIZE > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        current_call_frame
            .stack
            .push(U256::from(current_call_frame.calldata.len()))?;
        self.env.consumed_gas += gas_cost::CALLDATASIZE;

        Ok(OpcodeSuccess::Continue)
    }

    // CALLDATACOPY operation
    pub fn op_calldatacopy(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let dest_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);
        let calldata_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);
        let size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);

        let minimum_word_size = (size + WORD_SIZE - 1) / WORD_SIZE;
        let memory_expansion_cost = current_call_frame
            .memory
            .expansion_cost(dest_offset + size)?;
        let gas_cost = gas_cost::CALLDATACOPY_STATIC
            + gas_cost::CALLDATACOPY_DYNAMIC_BASE * minimum_word_size
            + memory_expansion_cost;

        if self.env.consumed_gas + gas_cost > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        self.env.consumed_gas += gas_cost;

        if size == 0 {
            return Ok(OpcodeSuccess::Continue);
        }

        let data = current_call_frame
            .calldata
            .slice(calldata_offset..calldata_offset + size);
        current_call_frame.memory.store_bytes(dest_offset, &data);

        Ok(OpcodeSuccess::Continue)
    }

    // RETURNDATASIZE operation
    pub fn op_returndatasize(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        if self.env.consumed_gas + gas_cost::RETURNDATASIZE > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        current_call_frame
            .stack
            .push(U256::from(current_call_frame.returndata.len()))?;
        self.env.consumed_gas += gas_cost::RETURNDATASIZE;

        Ok(OpcodeSuccess::Continue)
    }

    // RETURNDATACOPY operation
    pub fn op_returndatacopy(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let dest_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);
        let returndata_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);
        let size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .unwrap_or(usize::MAX);

        let minimum_word_size = (size + WORD_SIZE - 1) / WORD_SIZE;
        let memory_expansion_cost = current_call_frame
            .memory
            .expansion_cost(dest_offset + size)?;
        let gas_cost = gas_cost::RETURNDATACOPY_STATIC
            + gas_cost::RETURNDATACOPY_DYNAMIC_BASE * minimum_word_size
            + memory_expansion_cost;

        if self.env.consumed_gas + gas_cost > self.env.gas_limit {
            return Err(VMError::OutOfGas);
        }

        self.env.consumed_gas += gas_cost;

        if size == 0 {
            return Ok(OpcodeSuccess::Continue);
        }

        let data = current_call_frame
            .returndata
            .slice(returndata_offset..returndata_offset + size);
        current_call_frame.memory.store_bytes(dest_offset, &data);

        Ok(OpcodeSuccess::Continue)
    }
}
