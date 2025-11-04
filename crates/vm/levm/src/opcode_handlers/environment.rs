use crate::{
    errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError},
    gas_cost::{self},
    memory::calculate_memory_size,
    utils::{size_offset_to_usize, u256_to_usize, word_to_address},
    vm::VM,
};
use ethrex_common::{U256, utils::u256_from_big_endian_const};

// Environmental Information (16)
// Opcodes: ADDRESS, BALANCE, ORIGIN, CALLER, CALLVALUE, CALLDATALOAD, CALLDATASIZE, CALLDATACOPY, CODESIZE, CODECOPY, GASPRICE, EXTCODESIZE, EXTCODECOPY, RETURNDATASIZE, RETURNDATACOPY, EXTCODEHASH

impl<'a> VM<'a> {
    // ADDRESS operation
    pub fn op_address(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::ADDRESS)?;

        let addr = self.current_call_frame.to; // The recipient of the current call.

        self.current_stack()
            .push1(u256_from_big_endian_const(addr.to_fixed_bytes()))?;

        Ok(OpcodeResult::Continue)
    }

    // BALANCE operation
    pub fn op_balance(&mut self) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(self.current_stack().pop1()?);

        let address_was_cold = !self.substate.add_accessed_address(address);
        let account_balance = self.db.get_account(address)?.info.balance;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::balance(address_was_cold)?)?;

        self.current_stack().push1(account_balance)?;

        Ok(OpcodeResult::Continue)
    }

    // ORIGIN operation
    pub fn op_origin(&mut self) -> Result<OpcodeResult, VMError> {
        let origin = self.env.origin;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::ORIGIN)?;

        self.current_stack()
            .push1(u256_from_big_endian_const(origin.to_fixed_bytes()))?;

        Ok(OpcodeResult::Continue)
    }

    // CALLER operation
    pub fn op_caller(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::CALLER)?;

        let caller =
            u256_from_big_endian_const(self.current_call_frame.msg_sender.to_fixed_bytes());
        self.current_stack().push1(caller)?;

        Ok(OpcodeResult::Continue)
    }

    // CALLVALUE operation
    pub fn op_callvalue(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::CALLVALUE)?;

        let callvalue = self.current_call_frame.msg_value;

        self.current_stack().push1(callvalue)?;

        Ok(OpcodeResult::Continue)
    }

    // CALLDATALOAD operation
    pub fn op_calldataload(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::CALLDATALOAD)?;

        let calldata_size: U256 = self.current_call_frame.calldata.len().into();

        let offset = self.current_stack().pop1()?;

        // If the offset is larger than the actual calldata, then you
        // have no data to return.
        if offset > calldata_size {
            self.current_stack().push_zero()?;
            return Ok(OpcodeResult::Continue);
        };
        let offset: usize = offset
            .try_into()
            .map_err(|_| InternalError::TypeConversion)?;

        // All bytes after the end of the calldata are set to 0.
        let mut data = [0u8; 32];
        let size = 32;

        if offset < self.current_call_frame.calldata.len() {
            let diff = self.current_call_frame.calldata.len().wrapping_sub(offset);
            let final_size = size.min(diff);
            let end = offset.wrapping_add(final_size);

            #[expect(unsafe_code, reason = "bounds checked beforehand")]
            unsafe {
                data.get_unchecked_mut(..final_size)
                    .copy_from_slice(self.current_call_frame.calldata.get_unchecked(offset..end));
            }
        }

        let result = u256_from_big_endian_const(data);

        self.current_stack().push1(result)?;

        Ok(OpcodeResult::Continue)
    }

    // CALLDATASIZE operation
    pub fn op_calldatasize(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::CALLDATASIZE)?;

        let calldata_size = U256::from(self.current_call_frame.calldata.len());

        self.current_stack().push1(calldata_size)?;

        Ok(OpcodeResult::Continue)
    }

    // CALLDATACOPY operation
    pub fn op_calldatacopy(&mut self) -> Result<OpcodeResult, VMError> {
        let [dest_offset, calldata_offset, size] = *self.current_stack().pop()?;
        let (size, dest_offset) = size_offset_to_usize(size, dest_offset)?;
        let calldata_offset = u256_to_usize(calldata_offset).unwrap_or(usize::MAX);

        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::calldatacopy(
                new_memory_size,
                self.current_call_frame.memory.len(),
                size,
            )?)?;

        if size == 0 {
            return Ok(OpcodeResult::Continue);
        }

        let calldata_len = self.current_call_frame.calldata.len();

        // offset is out of bounds, so fill zeroes
        if calldata_offset >= calldata_len {
            self.current_call_frame
                .memory
                .store_zeros(dest_offset, size)?;
            return Ok(OpcodeResult::Continue);
        }

        #[expect(
            clippy::arithmetic_side_effects,
            clippy::indexing_slicing,
            reason = "bounds checked"
        )]
        {
            // we already verified calldata_len >= calldata_offset
            let available_data = calldata_len - calldata_offset;
            let copy_size = size.min(available_data);
            let zero_fill_size = size - copy_size;

            if zero_fill_size == 0 {
                // no zero padding needed

                // calldata_offset + copy_size can't overflow because its the min of size and (calldata_len - calldata_offset).
                let src_slice =
                    &self.current_call_frame.calldata[calldata_offset..calldata_offset + copy_size];
                self.current_call_frame
                    .memory
                    .store_data(dest_offset, src_slice)?;
            } else {
                let mut data = vec![0u8; size];

                let available_data = calldata_len - calldata_offset;
                let copy_size = size.min(available_data);

                if copy_size > 0 {
                    data[..copy_size].copy_from_slice(
                        &self.current_call_frame.calldata
                            [calldata_offset..calldata_offset + copy_size],
                    );
                }

                self.current_call_frame
                    .memory
                    .store_data(dest_offset, &data)?;
            }

            Ok(OpcodeResult::Continue)
        }
    }

    // CODESIZE operation
    pub fn op_codesize(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::CODESIZE)?;

        let code_size = U256::from(self.current_call_frame.bytecode.bytecode.len());

        self.current_stack().push1(code_size)?;

        Ok(OpcodeResult::Continue)
    }

    // CODECOPY operation
    pub fn op_codecopy(&mut self) -> Result<OpcodeResult, VMError> {
        let [dest_offset, code_offset, size] = *self.current_stack().pop()?;
        let (size, dest_offset) = size_offset_to_usize(size, dest_offset)?;
        let code_offset = u256_to_usize(code_offset).unwrap_or(usize::MAX);

        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::codecopy(
                new_memory_size,
                self.current_call_frame.memory.len(),
                size,
            )?)?;

        if size == 0 {
            return Ok(OpcodeResult::Continue);
        }

        // Happiest fast path, copy without an intermediate buffer because there is no need to pad 0s and also size doesn't overflow.
        if let Some(code_offset_end) = code_offset.checked_add(size)
            && code_offset_end <= self.current_call_frame.bytecode.bytecode.len()
        {
            #[expect(unsafe_code, reason = "bounds checked beforehand")]
            let slice = unsafe {
                self.current_call_frame
                    .bytecode
                    .bytecode
                    .get_unchecked(code_offset..code_offset_end)
            };
            self.current_call_frame
                .memory
                .store_data(dest_offset, slice)?;

            return Ok(OpcodeResult::Continue);
        }

        let mut data = vec![0u8; size];
        if code_offset < self.current_call_frame.bytecode.bytecode.len() {
            let diff = self
                .current_call_frame
                .bytecode
                .bytecode
                .len()
                .wrapping_sub(code_offset);
            let final_size = size.min(diff);
            let end = code_offset.wrapping_add(final_size);

            #[expect(unsafe_code, reason = "bounds checked beforehand")]
            unsafe {
                data.get_unchecked_mut(..final_size).copy_from_slice(
                    self.current_call_frame
                        .bytecode
                        .bytecode
                        .get_unchecked(code_offset..end),
                );
            }
        }

        self.current_call_frame
            .memory
            .store_data(dest_offset, &data)?;

        Ok(OpcodeResult::Continue)
    }

    // GASPRICE operation
    pub fn op_gasprice(&mut self) -> Result<OpcodeResult, VMError> {
        let gas_price = self.env.gas_price;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::GASPRICE)?;

        self.current_stack().push1(gas_price)?;

        Ok(OpcodeResult::Continue)
    }

    // EXTCODESIZE operation
    pub fn op_extcodesize(&mut self) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(self.current_stack().pop1()?);
        let address_was_cold = !self.substate.add_accessed_address(address);
        // FIXME: a bit wasteful to fetch the whole code just to get the length.
        let account_code_length = self.db.get_account_code(address)?.bytecode.len().into();

        self.current_call_frame
            .increase_consumed_gas(gas_cost::extcodesize(address_was_cold)?)?;

        self.current_stack().push1(account_code_length)?;

        Ok(OpcodeResult::Continue)
    }

    // EXTCODECOPY operation
    pub fn op_extcodecopy(&mut self) -> Result<OpcodeResult, VMError> {
        let [address, dest_offset, offset, size] = *self.current_stack().pop()?;

        let address = word_to_address(address);
        let (size, dest_offset) = size_offset_to_usize(size, dest_offset)?;
        let offset = u256_to_usize(offset).unwrap_or(usize::MAX);

        let current_memory_size = self.current_call_frame.memory.len();
        let address_was_cold = !self.substate.add_accessed_address(address);
        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::extcodecopy(
                size,
                new_memory_size,
                current_memory_size,
                address_was_cold,
            )?)?;

        if size == 0 {
            return Ok(OpcodeResult::Continue);
        }

        // If the bytecode is a delegation designation, it will copy the marker (0xef0100) || address.
        // https://eips.ethereum.org/EIPS/eip-7702#delegation-designation
        let bytecode = self.db.get_account_code(address)?;

        // Happiest fast path, copy without an intermediate buffer because there is no need to pad 0s and also size doesn't overflow.
        if let Some(offset_end) = offset.checked_add(size)
            && offset_end <= bytecode.bytecode.len()
        {
            #[expect(unsafe_code, reason = "bounds checked beforehand")]
            let slice = unsafe { bytecode.bytecode.get_unchecked(offset..offset_end) };
            self.current_call_frame
                .memory
                .store_data(dest_offset, slice)?;

            return Ok(OpcodeResult::Continue);
        }

        let mut data = vec![0u8; size];
        if offset < bytecode.bytecode.len() {
            let diff = bytecode.bytecode.len().wrapping_sub(offset);
            let final_size = size.min(diff);
            let end = offset.wrapping_add(final_size);

            #[expect(unsafe_code, reason = "bounds checked beforehand")]
            unsafe {
                data.get_unchecked_mut(..final_size)
                    .copy_from_slice(bytecode.bytecode.get_unchecked(offset..end));
            }
        }

        self.current_call_frame
            .memory
            .store_data(dest_offset, &data)?;

        Ok(OpcodeResult::Continue)
    }

    // RETURNDATASIZE operation
    pub fn op_returndatasize(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::RETURNDATASIZE)?;

        let returndata_size = U256::from(self.current_call_frame.sub_return_data.len());

        self.current_stack().push1(returndata_size)?;

        Ok(OpcodeResult::Continue)
    }

    // RETURNDATACOPY operation
    pub fn op_returndatacopy(&mut self) -> Result<OpcodeResult, VMError> {
        let [dest_offset, returndata_offset, size] = *self.current_stack().pop()?;

        let (size, dest_offset) = size_offset_to_usize(size, dest_offset)?;
        let returndata_offset =
            u256_to_usize(returndata_offset).map_err(|_| ExceptionalHalt::OutOfBounds)?;

        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::returndatacopy(
                new_memory_size,
                self.current_call_frame.memory.len(),
                size,
            )?)?;

        if size == 0 && returndata_offset == 0 {
            return Ok(OpcodeResult::Continue);
        }

        let sub_return_data_len = self.current_call_frame.sub_return_data.len();

        let copy_limit = returndata_offset
            .checked_add(size)
            .ok_or(ExceptionalHalt::VeryLargeNumber)?;

        if copy_limit > sub_return_data_len {
            return Err(ExceptionalHalt::OutOfBounds.into());
        }

        #[expect(unsafe_code, reason = "bounds checked beforehand")]
        let slice = unsafe {
            self.current_call_frame
                .sub_return_data
                .get_unchecked(returndata_offset..copy_limit)
        };
        self.current_call_frame
            .memory
            .store_data(dest_offset, slice)?;

        Ok(OpcodeResult::Continue)
    }

    // EXTCODEHASH operation
    pub fn op_extcodehash(&mut self) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(self.current_stack().pop1()?);
        let address_was_cold = !self.substate.add_accessed_address(address);
        let account = self.db.get_account(address)?;
        let account_is_empty = account.is_empty();
        let account_code_hash = account.info.code_hash.0;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::extcodehash(address_was_cold)?)?;

        // An account is considered empty when it has no code and zero nonce and zero balance. [EIP-161]
        if account_is_empty {
            self.current_stack().push_zero()?;
            return Ok(OpcodeResult::Continue);
        }

        let hash = u256_from_big_endian_const(account_code_hash);
        self.current_stack().push1(hash)?;

        Ok(OpcodeResult::Continue)
    }
}
