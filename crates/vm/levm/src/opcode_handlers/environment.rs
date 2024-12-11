use crate::{
    call_frame::CallFrame,
    errors::{InternalError, OpcodeSuccess, VMError},
    gas_cost,
    memory::{self, calculate_memory_size},
    vm::{word_to_address, VM},
};
use ethrex_core::U256;
use keccak_hash::keccak;

// Environmental Information (16)
// Opcodes: ADDRESS, BALANCE, ORIGIN, CALLER, CALLVALUE, CALLDATALOAD, CALLDATASIZE, CALLDATACOPY, CODESIZE, CODECOPY, GASPRICE, EXTCODESIZE, EXTCODECOPY, RETURNDATASIZE, RETURNDATACOPY, EXTCODEHASH

impl VM {
    // ADDRESS operation
    pub fn op_address(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::ADDRESS)?;

        let addr = current_call_frame.to; // The recipient of the current call.

        current_call_frame.stack.push(U256::from(addr.as_bytes()))?;

        Ok(OpcodeSuccess::Continue)
    }

    // BALANCE operation
    pub fn op_balance(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let address = word_to_address(current_call_frame.stack.pop()?);

        let (account_info, address_was_cold) = self.access_account(address);

        self.increase_consumed_gas(current_call_frame, gas_cost::balance(address_was_cold)?)?;

        current_call_frame.stack.push(account_info.balance)?;

        Ok(OpcodeSuccess::Continue)
    }

    // ORIGIN operation
    pub fn op_origin(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::ORIGIN)?;

        let origin = self.env.origin;
        current_call_frame
            .stack
            .push(U256::from(origin.as_bytes()))?;

        Ok(OpcodeSuccess::Continue)
    }

    // CALLER operation
    pub fn op_caller(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::CALLER)?;

        let caller = current_call_frame.msg_sender;
        current_call_frame
            .stack
            .push(U256::from(caller.as_bytes()))?;

        Ok(OpcodeSuccess::Continue)
    }

    // CALLVALUE operation
    pub fn op_callvalue(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::CALLVALUE)?;

        let callvalue = current_call_frame.msg_value;

        current_call_frame.stack.push(callvalue)?;

        Ok(OpcodeSuccess::Continue)
    }

    // CALLDATALOAD operation
    pub fn op_calldataload(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::CALLDATALOAD)?;

        let calldata_size: U256 = current_call_frame.calldata.len().into();

        let offset = current_call_frame.stack.pop()?;

        // If the offset is larger than the actual calldata, then you
        // have no data to return.
        if offset > calldata_size {
            current_call_frame.stack.push(U256::zero())?;
            return Ok(OpcodeSuccess::Continue);
        };
        let offset: usize = offset
            .try_into()
            .map_err(|_| VMError::Internal(InternalError::ConversionError))?;

        // All bytes after the end of the calldata are set to 0.
        let mut data = [0u8; 32];
        for (i, byte) in current_call_frame
            .calldata
            .iter()
            .skip(offset)
            .take(32)
            .enumerate()
        {
            if let Some(data_byte) = data.get_mut(i) {
                *data_byte = *byte;
            }
        }
        let result = U256::from_big_endian(&data);

        current_call_frame.stack.push(result)?;

        Ok(OpcodeSuccess::Continue)
    }

    // CALLDATASIZE operation
    pub fn op_calldatasize(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::CALLDATASIZE)?;

        current_call_frame
            .stack
            .push(U256::from(current_call_frame.calldata.len()))?;

        Ok(OpcodeSuccess::Continue)
    }

    // CALLDATACOPY operation
    pub fn op_calldatacopy(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let dest_offset = current_call_frame.stack.pop()?;
        let calldata_offset = current_call_frame.stack.pop()?;
        let size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::calldatacopy(new_memory_size, current_call_frame.memory.len(), size)?,
        )?;

        if size == 0 {
            return Ok(OpcodeSuccess::Continue);
        }

        let mut data = vec![0u8; size];
        if calldata_offset > current_call_frame.calldata.len().into() {
            memory::try_store_data(&mut current_call_frame.memory, dest_offset, &data)?;
            return Ok(OpcodeSuccess::Continue);
        }

        let calldata_offset: usize = calldata_offset
            .try_into()
            .map_err(|_err| VMError::Internal(InternalError::ConversionError))?;

        for (i, byte) in current_call_frame
            .calldata
            .iter()
            .skip(calldata_offset)
            .take(size)
            .enumerate()
        {
            if let Some(data_byte) = data.get_mut(i) {
                *data_byte = *byte;
            }
        }

        memory::try_store_data(&mut current_call_frame.memory, dest_offset, &data)?;

        Ok(OpcodeSuccess::Continue)
    }

    // CODESIZE operation
    pub fn op_codesize(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::CODESIZE)?;

        current_call_frame
            .stack
            .push(U256::from(current_call_frame.bytecode.len()))?;

        Ok(OpcodeSuccess::Continue)
    }

    // CODECOPY operation
    pub fn op_codecopy(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let destination_offset = current_call_frame.stack.pop()?;

        let code_offset = current_call_frame.stack.pop()?;

        let size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;

        let new_memory_size = calculate_memory_size(destination_offset, size)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::codecopy(new_memory_size, current_call_frame.memory.len(), size)?,
        )?;

        if size == 0 {
            return Ok(OpcodeSuccess::Continue);
        }

        let mut data = vec![0u8; size];
        if code_offset < current_call_frame.bytecode.len().into() {
            let code_offset: usize = code_offset
                .try_into()
                .map_err(|_| VMError::Internal(InternalError::ConversionError))?;

            for (i, byte) in current_call_frame
                .bytecode
                .iter()
                .skip(code_offset)
                .take(size)
                .enumerate()
            {
                if let Some(data_byte) = data.get_mut(i) {
                    *data_byte = *byte;
                }
            }
        }

        memory::try_store_data(&mut current_call_frame.memory, destination_offset, &data)?;

        Ok(OpcodeSuccess::Continue)
    }

    // GASPRICE operation
    pub fn op_gasprice(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::GASPRICE)?;

        current_call_frame.stack.push(self.env.gas_price)?;

        Ok(OpcodeSuccess::Continue)
    }

    // EXTCODESIZE operation
    pub fn op_extcodesize(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let address = word_to_address(current_call_frame.stack.pop()?);

        let (account_info, address_was_cold) = self.access_account(address);

        self.increase_consumed_gas(current_call_frame, gas_cost::extcodesize(address_was_cold)?)?;

        current_call_frame
            .stack
            .push(account_info.bytecode.len().into())?;

        Ok(OpcodeSuccess::Continue)
    }

    // EXTCODECOPY operation
    pub fn op_extcodecopy(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let address = word_to_address(current_call_frame.stack.pop()?);
        let dest_offset = current_call_frame.stack.pop()?;
        let offset = current_call_frame.stack.pop()?;
        let size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;

        let (account_info, address_was_cold) = self.access_account(address);

        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::extcodecopy(
                size,
                new_memory_size,
                current_call_frame.memory.len(),
                address_was_cold,
            )?,
        )?;

        if size == 0 {
            return Ok(OpcodeSuccess::Continue);
        }

        let mut data = vec![0u8; size];
        if offset < account_info.bytecode.len().into() {
            let offset: usize = offset
                .try_into()
                .map_err(|_| VMError::Internal(InternalError::ConversionError))?;
            for (i, byte) in account_info
                .bytecode
                .iter()
                .skip(offset)
                .take(size)
                .enumerate()
            {
                if let Some(data_byte) = data.get_mut(i) {
                    *data_byte = *byte;
                }
            }
        }

        memory::try_store_data(&mut current_call_frame.memory, dest_offset, &data)?;

        Ok(OpcodeSuccess::Continue)
    }

    // RETURNDATASIZE operation
    pub fn op_returndatasize(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::RETURNDATASIZE)?;

        current_call_frame
            .stack
            .push(U256::from(current_call_frame.sub_return_data.len()))?;

        Ok(OpcodeSuccess::Continue)
    }

    // RETURNDATACOPY operation
    pub fn op_returndatacopy(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let dest_offset = current_call_frame.stack.pop()?;
        let returndata_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;

        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::returndatacopy(new_memory_size, current_call_frame.memory.len(), size)?,
        )?;

        if size == 0 {
            return Ok(OpcodeSuccess::Continue);
        }

        let sub_return_data_len = current_call_frame.sub_return_data.len();

        if returndata_offset >= sub_return_data_len {
            return Err(VMError::VeryLargeNumber); // Maybe can create a new error instead of using this one
        }

        let mut data = vec![0u8; size];
        for (i, byte) in current_call_frame
            .sub_return_data
            .iter()
            .skip(returndata_offset)
            .take(size)
            .enumerate()
        {
            if let Some(data_byte) = data.get_mut(i) {
                *data_byte = *byte;
            }
        }

        memory::try_store_data(&mut current_call_frame.memory, dest_offset, &data)?;

        Ok(OpcodeSuccess::Continue)
    }

    // EXTCODEHASH operation
    pub fn op_extcodehash(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let address = word_to_address(current_call_frame.stack.pop()?);

        let (account_info, address_was_cold) = self.access_account(address);

        self.increase_consumed_gas(current_call_frame, gas_cost::extcodehash(address_was_cold)?)?;

        current_call_frame.stack.push(U256::from_big_endian(
            keccak(account_info.bytecode).as_fixed_bytes(),
        ))?;

        Ok(OpcodeSuccess::Continue)
    }
}
