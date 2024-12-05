use crate::{
    call_frame::CallFrame,
    constants::WORD_SIZE_IN_BYTES_USIZE,
    errors::{OpcodeSuccess, OutOfGasError, VMError},
    gas_cost, memory,
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

        let offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;

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
        let dest_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let calldata_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;
        let size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::calldatacopy(
                dest_offset
                    .checked_add(size)
                    .ok_or(VMError::OutOfOffset)?
                    .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
                    .ok_or(VMError::OutOfOffset)?,
                current_call_frame.memory.len(),
                size,
            )?,
        )?;

        if size == 0 {
            return Ok(OpcodeSuccess::Continue);
        }

        let mut data = vec![0u8; size];
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
        if self
            .env
            .consumed_gas
            .checked_add(gas_cost::CODESIZE)
            .ok_or(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow))?
            > self.env.gas_limit
        {
            return Err(VMError::OutOfGas(OutOfGasError::MaxGasLimitExceeded));
        }

        current_call_frame
            .stack
            .push(U256::from(current_call_frame.bytecode.len()))?;

        self.increase_consumed_gas(current_call_frame, gas_cost::CODESIZE)?;

        Ok(OpcodeSuccess::Continue)
    }

    // CODECOPY operation
    pub fn op_codecopy(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let destination_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let code_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::codecopy(
                destination_offset
                    .checked_add(size)
                    .ok_or(VMError::OutOfOffset)?
                    .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
                    .ok_or(VMError::OutOfOffset)?,
                current_call_frame.memory.len(),
                size,
            )?,
        )?;

        if size == 0 {
            return Ok(OpcodeSuccess::Continue);
        }

        let mut data = vec![0u8; size];
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
        let dest_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
        let size: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;

        let (account_info, address_was_cold) = self.access_account(address);

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::extcodecopy(
                dest_offset
                    .checked_add(size)
                    .ok_or(VMError::OutOfOffset)?
                    .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
                    .ok_or(VMError::OutOfOffset)?,
                current_call_frame.memory.len(),
                address_was_cold,
            )?,
        )?;

        if size == 0 {
            return Ok(OpcodeSuccess::Continue);
        }

        let mut data = vec![0u8; size];
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
        let dest_offset: usize = current_call_frame
            .stack
            .pop()?
            .try_into()
            .map_err(|_| VMError::VeryLargeNumber)?;
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

        self.increase_consumed_gas(
            current_call_frame,
            gas_cost::returndatacopy(
                dest_offset
                    .checked_add(size)
                    .ok_or(VMError::OutOfOffset)?
                    .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
                    .ok_or(VMError::OutOfOffset)?,
                current_call_frame.memory.len(),
                size,
            )?,
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
