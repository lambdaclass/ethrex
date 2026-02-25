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
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::ADDRESS)?;

        let addr = current_call_frame.to; // The recipient of the current call.

        current_call_frame
            .stack
            .push(u256_from_big_endian_const(addr.to_fixed_bytes()))?;

        Ok(OpcodeResult::Continue)
    }

    // BALANCE operation
    pub fn op_balance(&mut self) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(self.current_call_frame.stack.pop1()?);
        let address_was_cold = !self.substate.add_accessed_address(address);

        // Gas check MUST pass before state access per EIP-7928:
        // "If pre-state validation fails, the target is never accessed and must not appear in the BAL."
        self.current_call_frame
            .increase_consumed_gas(gas_cost::balance(address_was_cold)?)?;

        // State access AFTER gas check passes
        let account_balance = self.db.get_account(address)?.info.balance;

        // Record address touch for BAL (after gas check passes)
        if let Some(recorder) = self.db.bal_recorder.as_mut() {
            recorder.record_touched_address(address);
        }

        self.current_call_frame.stack.push(account_balance)?;

        Ok(OpcodeResult::Continue)
    }

    // ORIGIN operation
    pub fn op_origin(&mut self) -> Result<OpcodeResult, VMError> {
        let origin = self.env.origin;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::ORIGIN)?;

        current_call_frame
            .stack
            .push(u256_from_big_endian_const(origin.to_fixed_bytes()))?;

        Ok(OpcodeResult::Continue)
    }

    // CALLER operation
    pub fn op_caller(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::CALLER)?;

        let caller = u256_from_big_endian_const(current_call_frame.msg_sender.to_fixed_bytes());
        current_call_frame.stack.push(caller)?;

        Ok(OpcodeResult::Continue)
    }

    // CALLVALUE operation
    pub fn op_callvalue(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::CALLVALUE)?;

        let callvalue = current_call_frame.msg_value;

        current_call_frame.stack.push(callvalue)?;

        Ok(OpcodeResult::Continue)
    }

    // CALLDATALOAD operation
    #[inline]
    pub fn op_calldataload(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::CALLDATALOAD)?;

        let calldata_size: U256 = current_call_frame.calldata.len().into();

        let offset = current_call_frame.stack.pop1()?;

        // If the offset is larger than the actual calldata, then you
        // have no data to return.
        if offset > calldata_size {
            current_call_frame.stack.push_zero()?;
            return Ok(OpcodeResult::Continue);
        };
        let offset: usize = offset
            .try_into()
            .map_err(|_| InternalError::TypeConversion)?;

        // All bytes after the end of the calldata are set to 0.
        let mut data = [0u8; 32];
        let size = 32;

        if offset < current_call_frame.calldata.len() {
            let diff = current_call_frame.calldata.len().wrapping_sub(offset);
            let final_size = size.min(diff);
            let end = offset.wrapping_add(final_size);

            #[expect(unsafe_code, reason = "bounds checked beforehand")]
            unsafe {
                data.get_unchecked_mut(..final_size)
                    .copy_from_slice(current_call_frame.calldata.get_unchecked(offset..end));
            }
        }

        let result = u256_from_big_endian_const(data);

        current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }

    // CALLDATASIZE operation
    pub fn op_calldatasize(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::CALLDATASIZE)?;

        current_call_frame
            .stack
            .push(U256::from(current_call_frame.calldata.len()))?;

        Ok(OpcodeResult::Continue)
    }

    // CALLDATACOPY operation
    #[expect(clippy::arithmetic_side_effects, reason = "bound checked")]
    pub fn op_calldatacopy(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        let [dest_offset, calldata_offset, size] = *current_call_frame.stack.pop()?;
        let (size, dest_offset) = size_offset_to_usize(size, dest_offset)?;
        let calldata_offset = u256_to_usize(calldata_offset).unwrap_or(usize::MAX);

        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        current_call_frame.increase_consumed_gas(gas_cost::calldatacopy(
            new_memory_size,
            current_call_frame.memory.len(),
            size,
        )?)?;

        if size == 0 {
            return Ok(OpcodeResult::Continue);
        }

        let calldata_len = current_call_frame.calldata.len();

        // offset is out of bounds, so fill zeroes
        if calldata_offset >= calldata_len {
            current_call_frame
                .memory
                .store_data_zero_padded(dest_offset, &[], size)?;
            return Ok(OpcodeResult::Continue);
        }

        // We already verified calldata_len >= calldata_offset.
        let available_data = calldata_len - calldata_offset;
        let copy_size = size.min(available_data);
        #[expect(clippy::indexing_slicing, reason = "bounds checked")]
        let src_slice = &current_call_frame.calldata[calldata_offset..calldata_offset + copy_size];
        current_call_frame
            .memory
            .store_data_zero_padded(dest_offset, src_slice, size)?;

        Ok(OpcodeResult::Continue)
    }

    // CODESIZE operation
    pub fn op_codesize(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::CODESIZE)?;

        current_call_frame
            .stack
            .push(U256::from(current_call_frame.bytecode.code_len))?;

        Ok(OpcodeResult::Continue)
    }

    // CODECOPY operation
    pub fn op_codecopy(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;

        let [dest_offset, code_offset, size] = *current_call_frame.stack.pop()?;
        let (size, dest_offset) = size_offset_to_usize(size, dest_offset)?;
        let code_offset = u256_to_usize(code_offset).unwrap_or(usize::MAX);

        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        current_call_frame.increase_consumed_gas(gas_cost::codecopy(
            new_memory_size,
            current_call_frame.memory.len(),
            size,
        )?)?;

        if size == 0 {
            return Ok(OpcodeResult::Continue);
        }

        // Happiest fast path, copy without an intermediate buffer because there is no need to pad 0s and also size doesn't overflow.
        if let Some(code_offset_end) = code_offset.checked_add(size)
            && code_offset_end <= current_call_frame.bytecode.code_len
        {
            #[expect(unsafe_code, reason = "bounds checked beforehand")]
            let slice = unsafe {
                current_call_frame
                    .bytecode
                    .bytecode
                    .get_unchecked(code_offset..code_offset_end)
            };
            current_call_frame.memory.store_data(dest_offset, slice)?;

            return Ok(OpcodeResult::Continue);
        }

        let code_len = current_call_frame.bytecode.code_len;

        #[expect(clippy::arithmetic_side_effects)]
        let slice = if code_offset < code_len {
            let available_data = code_len - code_offset;
            let copy_size = size.min(available_data);
            let end = code_offset + copy_size;
            #[expect(unsafe_code, reason = "bounds checked beforehand")]
            unsafe {
                current_call_frame
                    .bytecode
                    .bytecode
                    .get_unchecked(code_offset..end)
            }
        } else {
            &[]
        };

        current_call_frame
            .memory
            .store_data_zero_padded(dest_offset, slice, size)?;

        Ok(OpcodeResult::Continue)
    }

    // GASPRICE operation
    pub fn op_gasprice(&mut self) -> Result<OpcodeResult, VMError> {
        let gas_price = self.env.gas_price;
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::GASPRICE)?;

        current_call_frame.stack.push(gas_price)?;

        Ok(OpcodeResult::Continue)
    }

    // EXTCODESIZE operation
    pub fn op_extcodesize(&mut self) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(self.current_call_frame.stack.pop1()?);
        let address_was_cold = !self.substate.add_accessed_address(address);

        // Gas check MUST pass before state access per EIP-7928:
        // "If pre-state validation fails, the target is never accessed and must not appear in the BAL."
        self.current_call_frame
            .increase_consumed_gas(gas_cost::extcodesize(address_was_cold)?)?;

        // State access AFTER gas check passes (using optimized code length lookup)
        let account_code_length = self.db.get_code_length(address)?.into();

        // Record address touch for BAL (after gas check passes)
        if let Some(recorder) = self.db.bal_recorder.as_mut() {
            recorder.record_touched_address(address);
        }

        self.current_call_frame.stack.push(account_code_length)?;

        Ok(OpcodeResult::Continue)
    }

    // EXTCODECOPY operation
    pub fn op_extcodecopy(&mut self) -> Result<OpcodeResult, VMError> {
        let call_frame = &mut self.current_call_frame;
        let [address, dest_offset, offset, size] = *call_frame.stack.pop()?;

        let address = word_to_address(address);
        let (size, dest_offset) = size_offset_to_usize(size, dest_offset)?;
        let offset = u256_to_usize(offset).unwrap_or(usize::MAX);

        let current_memory_size = call_frame.memory.len();
        let address_was_cold = !self.substate.add_accessed_address(address);
        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        // Gas check MUST pass before recording address in BAL per EIP-7928:
        // "If pre-state validation fails, the target is never accessed and must not appear in the BAL."
        self.current_call_frame
            .increase_consumed_gas(gas_cost::extcodecopy(
                size,
                new_memory_size,
                current_memory_size,
                address_was_cold,
            )?)?;

        // Record address touch for BAL (after gas check passes)
        if let Some(recorder) = self.db.bal_recorder.as_mut() {
            recorder.record_touched_address(address);
        }

        if size == 0 {
            return Ok(OpcodeResult::Continue);
        }

        // If the bytecode is a delegation designation, it will copy the marker (0xef0100) || address.
        // https://eips.ethereum.org/EIPS/eip-7702#delegation-designation
        let bytecode = self.db.get_account_code(address)?;

        // Happiest fast path, copy without an intermediate buffer because there is no need to pad 0s and also size doesn't overflow.
        if let Some(offset_end) = offset.checked_add(size)
            && offset_end <= bytecode.code_len
        {
            #[expect(unsafe_code, reason = "bounds checked beforehand")]
            let slice = unsafe { bytecode.bytecode.get_unchecked(offset..offset_end) };
            self.current_call_frame
                .memory
                .store_data(dest_offset, slice)?;

            return Ok(OpcodeResult::Continue);
        }

        let code_len = bytecode.code_len;

        #[expect(clippy::arithmetic_side_effects)]
        let slice = if offset < code_len {
            let available_data = code_len - offset;
            let copy_size = size.min(available_data);
            let end = offset + copy_size;
            #[expect(unsafe_code, reason = "bounds checked beforehand")]
            unsafe {
                bytecode.bytecode.get_unchecked(offset..end)
            }
        } else {
            &[]
        };

        self.current_call_frame
            .memory
            .store_data_zero_padded(dest_offset, slice, size)?;

        Ok(OpcodeResult::Continue)
    }

    // RETURNDATASIZE operation
    pub fn op_returndatasize(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::RETURNDATASIZE)?;

        current_call_frame
            .stack
            .push(U256::from(current_call_frame.sub_return_data.len()))?;

        Ok(OpcodeResult::Continue)
    }

    // RETURNDATACOPY operation
    pub fn op_returndatacopy(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        let [dest_offset, returndata_offset, size] = *current_call_frame.stack.pop()?;

        let (size, dest_offset) = size_offset_to_usize(size, dest_offset)?;
        let returndata_offset =
            u256_to_usize(returndata_offset).map_err(|_| ExceptionalHalt::OutOfBounds)?;

        let new_memory_size = calculate_memory_size(dest_offset, size)?;

        current_call_frame.increase_consumed_gas(gas_cost::returndatacopy(
            new_memory_size,
            current_call_frame.memory.len(),
            size,
        )?)?;

        if size == 0 && returndata_offset == 0 {
            return Ok(OpcodeResult::Continue);
        }

        let sub_return_data_len = current_call_frame.sub_return_data.len();

        let copy_limit = returndata_offset
            .checked_add(size)
            .ok_or(ExceptionalHalt::VeryLargeNumber)?;

        if copy_limit > sub_return_data_len {
            return Err(ExceptionalHalt::OutOfBounds.into());
        }

        #[expect(unsafe_code, reason = "bounds checked beforehand")]
        let slice = unsafe {
            current_call_frame
                .sub_return_data
                .get_unchecked(returndata_offset..copy_limit)
        };
        current_call_frame.memory.store_data(dest_offset, slice)?;

        Ok(OpcodeResult::Continue)
    }

    // EXTCODEHASH operation
    pub fn op_extcodehash(&mut self) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(self.current_call_frame.stack.pop1()?);
        let address_was_cold = !self.substate.add_accessed_address(address);

        // Gas check MUST pass before state access per EIP-7928:
        // "If pre-state validation fails, the target is never accessed and must not appear in the BAL."
        self.current_call_frame
            .increase_consumed_gas(gas_cost::extcodehash(address_was_cold)?)?;

        // State access AFTER gas check passes
        let account = self.db.get_account(address)?;
        let account_is_empty = account.is_empty();
        let account_code_hash = account.info.code_hash.0;

        // Record address touch for BAL (after gas check passes)
        if let Some(recorder) = self.db.bal_recorder.as_mut() {
            recorder.record_touched_address(address);
        }

        // An account is considered empty when it has no code and zero nonce and zero balance. [EIP-161]
        if account_is_empty {
            self.current_call_frame.stack.push_zero()?;
            return Ok(OpcodeResult::Continue);
        }

        let hash = u256_from_big_endian_const(account_code_hash);
        self.current_call_frame.stack.push(hash)?;

        Ok(OpcodeResult::Continue)
    }
}
