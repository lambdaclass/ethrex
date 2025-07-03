use crate::{
    errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError},
    gas_cost::{self},
    memory::{self, calculate_memory_size},
    utils::word_to_address,
    vm::VM,
};
use ethrex_common::{U256, utils::u256_from_h160};

// Environmental Information (16)
// Opcodes: ADDRESS, BALANCE, ORIGIN, CALLER, CALLVALUE, CALLDATALOAD, CALLDATASIZE, CALLDATACOPY, CODESIZE, CODECOPY, GASPRICE, EXTCODESIZE, EXTCODECOPY, RETURNDATASIZE, RETURNDATACOPY, EXTCODEHASH

impl<'a> VM<'a> {
    // ADDRESS operation
    pub fn op_address(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::ADDRESS)?;

        let addr = current_call_frame.to; // The recipient of the current call.

        current_call_frame.stack.push(&[u256_from_h160(addr)])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // BALANCE operation
    pub fn op_balance(&mut self) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(self.current_call_frame_mut()?.stack.pop::<1>()?[0]);

        let (account, address_was_cold) = self.db.access_account(&mut self.substate, address)?;
        let account_balance = account.info.balance;

        let current_call_frame = self.current_call_frame_mut()?;

        current_call_frame.increase_consumed_gas(gas_cost::balance(address_was_cold)?)?;

        current_call_frame.stack.push(&[account_balance])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // ORIGIN operation
    pub fn op_origin(&mut self) -> Result<OpcodeResult, VMError> {
        let origin = self.env.origin;
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::ORIGIN)?;

        current_call_frame.stack.push(&[u256_from_h160(origin)])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CALLER operation
    pub fn op_caller(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::CALLER)?;

        let caller = current_call_frame.msg_sender;
        current_call_frame.stack.push(&[u256_from_h160(caller)])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CALLVALUE operation
    pub fn op_callvalue(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::CALLVALUE)?;

        let callvalue = current_call_frame.msg_value;

        current_call_frame.stack.push(&[callvalue])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CALLDATALOAD operation
    pub fn op_calldataload(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::CALLDATALOAD)?;

        #[expect(clippy::as_conversions)]
        let calldata_size: U256 = (current_call_frame.calldata.len() as u64).into();

        let [offset] = *current_call_frame.stack.pop()?;

        // If the offset is larger than the actual calldata, then you
        // have no data to return.
        if offset > calldata_size {
            current_call_frame.stack.push(&[U256::ZERO])?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        };
        let offset: usize = offset
            .try_into()
            .map_err(|_| InternalError::TypeConversion)?;

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
        let result = U256::from_be_bytes(data);

        current_call_frame.stack.push(&[result])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CALLDATASIZE operation
    pub fn op_calldatasize(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::CALLDATASIZE)?;

        #[expect(clippy::as_conversions)]
        current_call_frame
            .stack
            .push(&[U256::from(current_call_frame.calldata.len() as u64)])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CALLDATACOPY operation
    pub fn op_calldatacopy(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        let [dest_offset, calldata_offset, size] = *current_call_frame.stack.pop()?;
        let size: usize = size
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        current_call_frame.increase_consumed_gas(gas_cost::calldatacopy(
            new_memory_size,
            current_call_frame.memory.len(),
            size,
        )?)?;

        if size == 0 {
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let mut data = vec![0u8; size];
        #[expect(clippy::as_conversions)]
        if calldata_offset > U256::from(current_call_frame.calldata.len() as u64) {
            memory::try_store_data(&mut current_call_frame.memory, dest_offset, &data)?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let calldata_offset: usize = calldata_offset
            .try_into()
            .map_err(|_err| InternalError::TypeConversion)?;

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

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CODESIZE operation
    pub fn op_codesize(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::CODESIZE)?;

        #[expect(clippy::as_conversions)]
        current_call_frame
            .stack
            .push(&[U256::from(current_call_frame.bytecode.len() as u64)])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CODECOPY operation
    pub fn op_codecopy(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;

        let [destination_offset, code_offset, size] = *current_call_frame.stack.pop()?;

        let size: usize = size
            .try_into()
            .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;

        let new_memory_size = calculate_memory_size(destination_offset, size)?;

        current_call_frame.increase_consumed_gas(gas_cost::codecopy(
            new_memory_size,
            current_call_frame.memory.len(),
            size,
        )?)?;

        if size == 0 {
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let mut data = vec![0u8; size];
        #[expect(clippy::as_conversions)]
        if code_offset < U256::from(current_call_frame.bytecode.len() as u64) {
            let code_offset: usize = code_offset
                .try_into()
                .map_err(|_| InternalError::TypeConversion)?;

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

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // GASPRICE operation
    pub fn op_gasprice(&mut self) -> Result<OpcodeResult, VMError> {
        let gas_price = self.env.gas_price;
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::GASPRICE)?;

        current_call_frame.stack.push(&[gas_price])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // EXTCODESIZE operation
    pub fn op_extcodesize(&mut self) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(self.current_call_frame_mut()?.stack.pop::<1>()?[0]);

        let (account, address_was_cold) = self.db.access_account(&mut self.substate, address)?;

        #[expect(clippy::as_conversions)]
        let account_code_length = (account.code.len() as u64).into();

        let current_call_frame = self.current_call_frame_mut()?;

        current_call_frame.increase_consumed_gas(gas_cost::extcodesize(address_was_cold)?)?;

        current_call_frame.stack.push(&[account_code_length])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // EXTCODECOPY operation
    pub fn op_extcodecopy(&mut self) -> Result<OpcodeResult, VMError> {
        let call_frame = self.current_call_frame_mut()?;
        let [address, dest_offset, offset, size] = *call_frame.stack.pop()?;
        let address = word_to_address(address);
        let size = size
            .try_into()
            .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;
        let current_memory_size = call_frame.memory.len();

        let (_, address_was_cold) = self.db.access_account(&mut self.substate, address)?;

        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        self.current_call_frame_mut()?
            .increase_consumed_gas(gas_cost::extcodecopy(
                size,
                new_memory_size,
                current_memory_size,
                address_was_cold,
            )?)?;

        if size == 0 {
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        // If the bytecode is a delegation designation, it will copy the marker (0xef0100) || address.
        // https://eips.ethereum.org/EIPS/eip-7702#delegation-designation
        let bytecode = &self.db.get_account(address)?.code;

        let mut data = vec![0u8; size];
        #[expect(clippy::as_conversions)]
        if offset < U256::from(bytecode.len() as u64) {
            let offset: usize = offset
                .try_into()
                .map_err(|_| InternalError::TypeConversion)?;
            for (i, byte) in bytecode.iter().skip(offset).take(size).enumerate() {
                if let Some(data_byte) = data.get_mut(i) {
                    *data_byte = *byte;
                }
            }
        }

        memory::try_store_data(
            &mut self.current_call_frame_mut()?.memory,
            dest_offset,
            &data,
        )?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // RETURNDATASIZE operation
    pub fn op_returndatasize(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::RETURNDATASIZE)?;

        #[expect(clippy::as_conversions)]
        current_call_frame
            .stack
            .push(&[U256::from(current_call_frame.sub_return_data.len() as u64)])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // RETURNDATACOPY operation
    pub fn op_returndatacopy(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        let [dest_offset, returndata_offset, size] = *current_call_frame.stack.pop()?;
        let returndata_offset: usize = returndata_offset
            .try_into()
            .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;
        let size: usize = size
            .try_into()
            .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;

        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        current_call_frame.increase_consumed_gas(gas_cost::returndatacopy(
            new_memory_size,
            current_call_frame.memory.len(),
            size,
        )?)?;

        if size == 0 && returndata_offset == 0 {
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let sub_return_data_len = current_call_frame.sub_return_data.len();

        let copy_limit = returndata_offset
            .checked_add(size)
            .ok_or(ExceptionalHalt::VeryLargeNumber)?;

        if copy_limit > sub_return_data_len {
            return Err(ExceptionalHalt::OutOfBounds.into());
        }

        // Actually we don't need to fill with zeros for out of bounds bytes, this works but is overkill because of the previous validations.
        // I would've used copy_from_slice but it can panic.
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

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // EXTCODEHASH operation
    pub fn op_extcodehash(&mut self) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(self.current_call_frame_mut()?.stack.pop::<1>()?[0]);

        let (account_is_empty, account_code_hash, address_was_cold) = {
            let (account, address_was_cold) =
                self.db.access_account(&mut self.substate, address)?;
            (
                account.is_empty(),
                account.info.code_hash.0,
                address_was_cold,
            )
        };

        let current_call_frame = self.current_call_frame_mut()?;

        current_call_frame.increase_consumed_gas(gas_cost::extcodehash(address_was_cold)?)?;

        // An account is considered empty when it has no code and zero nonce and zero balance. [EIP-161]
        if account_is_empty {
            current_call_frame.stack.push(&[U256::ZERO])?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let hash = U256::from_be_bytes(account_code_hash);
        current_call_frame.stack.push(&[hash])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
