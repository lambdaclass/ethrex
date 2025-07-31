use crate::{
    errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError},
    gas_cost::{self},
    memory::calculate_memory_size,
    utils::word_to_address,
    vm::VM,
};
use ethrex_common::{U256, utils::u256_from_big_endian_const};

// Environmental Information (16)
// Opcodes: ADDRESS, BALANCE, ORIGIN, CALLER, CALLVALUE, CALLDATALOAD, CALLDATASIZE, CALLDATACOPY, CODESIZE, CODECOPY, GASPRICE, EXTCODESIZE, EXTCODECOPY, RETURNDATASIZE, RETURNDATACOPY, EXTCODEHASH

impl<'a> VM<'a> {
    // ADDRESS operation
    pub fn op_address(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::ADDRESS)?;

        let addr = current_call_frame.to; // The recipient of the current call.

        current_call_frame
            .stack
            .push1(u256_from_big_endian_const(addr.to_fixed_bytes()))?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // BALANCE operation
    pub fn op_balance(&mut self) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(self.current_call_frame.stack.pop1()?);

        let address_was_cold = self.substate.accessed_addresses.insert(address);
        let account_balance = self.db.get_account(address)?.info.balance;

        let current_call_frame = &mut self.current_call_frame;

        current_call_frame.increase_consumed_gas(gas_cost::balance(address_was_cold)?)?;

        current_call_frame.stack.push1(account_balance)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // ORIGIN operation
    pub fn op_origin(&mut self) -> Result<OpcodeResult, VMError> {
        let origin = self.env.origin;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::ORIGIN)?;

        current_call_frame
            .stack
            .push1(u256_from_big_endian_const(origin.to_fixed_bytes()))?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CALLER operation
    pub fn op_caller(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::CALLER)?;

        let caller = current_call_frame.msg_sender;
        current_call_frame
            .stack
            .push(&[u256_from_big_endian_const(caller.to_fixed_bytes())])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CALLVALUE operation
    pub fn op_callvalue(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::CALLVALUE)?;

        let callvalue = current_call_frame.msg_value;

        current_call_frame.stack.push1(callvalue)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CALLDATALOAD operation
    pub fn op_calldataload(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::CALLDATALOAD)?;

        let calldata_size: U256 = current_call_frame.calldata.len().into();

        let offset = current_call_frame.stack.pop1()?;

        // If the offset is larger than the actual calldata, then you
        // have no data to return.
        if offset > calldata_size {
            current_call_frame.stack.push1(U256::zero())?;
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
        let result = u256_from_big_endian_const(data);

        current_call_frame.stack.push1(result)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CALLDATASIZE operation
    pub fn op_calldatasize(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::CALLDATASIZE)?;

        current_call_frame
            .stack
            .push1(U256::from(current_call_frame.calldata.len()))?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CALLDATACOPY operation
    pub fn op_calldatacopy(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        let [dest_offset, calldata_offset, size] = *current_call_frame.stack.pop()?;
        let size: usize = size
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

        let dest_offset: usize = match dest_offset.try_into() {
            Ok(x) => x,
            Err(_) if size == 0 => 0,
            Err(_) => return Err(ExceptionalHalt::VeryLargeNumber.into()),
        };

        let calldata_offset: usize = match calldata_offset.try_into() {
            Ok(x) => x,
            Err(_) => usize::MAX,
        };

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

        if calldata_offset <= current_call_frame.calldata.len() {
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
        }

        current_call_frame.memory.store_data(dest_offset, &data)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CODESIZE operation
    pub fn op_codesize(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::CODESIZE)?;

        current_call_frame
            .stack
            .push1(U256::from(current_call_frame.bytecode.len()))?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // CODECOPY operation
    pub fn op_codecopy(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;

        let [destination_offset, code_offset, size] = *current_call_frame.stack.pop()?;

        let size: usize = size
            .try_into()
            .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;

        let destination_offset: usize = match destination_offset.try_into() {
            Ok(x) => x,
            Err(_) if size == 0 => 0,
            Err(_) => return Err(ExceptionalHalt::VeryLargeNumber.into()),
        };
        let code_offset: usize = match code_offset.try_into() {
            Ok(x) => x,
            Err(_) => usize::MAX,
        };

        let new_memory_size = calculate_memory_size(destination_offset, size)?;

        current_call_frame.increase_consumed_gas(gas_cost::codecopy(
            new_memory_size,
            current_call_frame.memory.len(),
            size,
        )?)?;

        if size == 0 {
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        // Happiest fast path, copy without an intermediate buffer because there is no need to pad 0s and also size doesn't overflow.
        if let Some(code_offset_end) = code_offset.checked_add(size) {
            if code_offset_end < current_call_frame.bytecode.len() {
                #[expect(unsafe_code, reason = "bounds checked beforehand")]
                let slice = unsafe {
                    current_call_frame
                        .bytecode
                        .get_unchecked(code_offset..code_offset_end)
                };
                current_call_frame
                    .memory
                    .store_data(destination_offset, slice)?;

                return Ok(OpcodeResult::Continue { pc_increment: 1 });
            }
        }

        let mut data = vec![0u8; size];
        if code_offset < current_call_frame.bytecode.len() {
            let diff = current_call_frame.bytecode.len().wrapping_sub(code_offset);
            let final_size = size.min(diff);
            let end = code_offset.wrapping_add(final_size);

            #[expect(unsafe_code, reason = "bounds checked beforehand")]
            unsafe {
                data.get_unchecked_mut(..final_size)
                    .copy_from_slice(current_call_frame.bytecode.get_unchecked(code_offset..end));
            }
        }

        current_call_frame
            .memory
            .store_data(destination_offset, &data)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // GASPRICE operation
    pub fn op_gasprice(&mut self) -> Result<OpcodeResult, VMError> {
        let gas_price = self.env.gas_price;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::GASPRICE)?;

        current_call_frame.stack.push1(gas_price)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // EXTCODESIZE operation
    pub fn op_extcodesize(&mut self) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(self.current_call_frame.stack.pop1()?);
        let address_was_cold = self.substate.accessed_addresses.insert(address);
        let account_code_length = self.db.get_account(address)?.code.len().into();

        let current_call_frame = &mut self.current_call_frame;

        current_call_frame.increase_consumed_gas(gas_cost::extcodesize(address_was_cold)?)?;

        current_call_frame.stack.push1(account_code_length)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // EXTCODECOPY operation
    pub fn op_extcodecopy(&mut self) -> Result<OpcodeResult, VMError> {
        let call_frame = &mut self.current_call_frame;
        let [address, dest_offset, offset, size] = *call_frame.stack.pop()?;
        let address = word_to_address(address);
        let size = size
            .try_into()
            .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;
        let current_memory_size = call_frame.memory.len();

        let address_was_cold = self.substate.accessed_addresses.insert(address);

        let dest_offset: usize = match dest_offset.try_into() {
            Ok(x) => x,
            Err(_) if size == 0 => 0,
            Err(_) => return Err(ExceptionalHalt::VeryLargeNumber.into()),
        };

        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        self.current_call_frame
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
        if offset < bytecode.len().into() {
            let offset: usize = offset
                .try_into()
                .map_err(|_| InternalError::TypeConversion)?;
            for (i, byte) in bytecode.iter().skip(offset).take(size).enumerate() {
                if let Some(data_byte) = data.get_mut(i) {
                    *data_byte = *byte;
                }
            }
        }

        self.current_call_frame
            .memory
            .store_data(dest_offset, &data)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // RETURNDATASIZE operation
    pub fn op_returndatasize(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::RETURNDATASIZE)?;

        current_call_frame
            .stack
            .push1(U256::from(current_call_frame.sub_return_data.len()))?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // RETURNDATACOPY operation
    pub fn op_returndatacopy(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        let [dest_offset, returndata_offset, size] = *current_call_frame.stack.pop()?;
        let returndata_offset: usize = returndata_offset
            .try_into()
            .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;
        let size: usize = size
            .try_into()
            .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;

        let dest_offset: usize = match dest_offset.try_into() {
            Ok(x) => x,
            Err(_) if size == 0 => 0,
            Err(_) => return Err(ExceptionalHalt::VeryLargeNumber.into()),
        };

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

        current_call_frame.memory.store_data(dest_offset, &data)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // EXTCODEHASH operation
    pub fn op_extcodehash(&mut self) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(self.current_call_frame.stack.pop1()?);
        let address_was_cold = self.substate.accessed_addresses.insert(address);
        let account = self.db.get_account(address)?;
        let account_is_empty = account.is_empty();
        let account_code_hash = account.info.code_hash.0;
        let current_call_frame = &mut self.current_call_frame;

        current_call_frame.increase_consumed_gas(gas_cost::extcodehash(address_was_cold)?)?;

        // An account is considered empty when it has no code and zero nonce and zero balance. [EIP-161]
        if account_is_empty {
            current_call_frame.stack.push1(U256::zero())?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let hash = u256_from_big_endian_const(account_code_hash);
        current_call_frame.stack.push1(hash)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
