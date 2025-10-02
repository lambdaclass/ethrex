use std::cell::OnceCell;

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
    pub fn op_address(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::ADDRESS)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let addr = self.current_call_frame.to; // The recipient of the current call.

        if let Err(err) = self
            .current_call_frame
            .stack
            .push1(u256_from_big_endian_const(addr.to_fixed_bytes()))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        OpcodeResult::Continue
    }

    // BALANCE operation
    pub fn op_balance(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let address = match self.current_call_frame.stack.pop1().map(word_to_address) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let address_was_cold = !self.substate.add_accessed_address(address);
        let account_balance = match self.db.get_account(address) {
            Ok(x) => x.info.balance,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = gas_cost::balance(address_was_cold)
            .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self.current_call_frame.stack.push1(account_balance) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // ORIGIN operation
    pub fn op_origin(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let origin = self.env.origin;

        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::ORIGIN)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self
            .current_call_frame
            .stack
            .push1(u256_from_big_endian_const(origin.to_fixed_bytes()))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // CALLER operation
    pub fn op_caller(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::CALLER)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let caller =
            u256_from_big_endian_const(self.current_call_frame.msg_sender.to_fixed_bytes());
        if let Err(err) = self.current_call_frame.stack.push1(caller) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // CALLVALUE operation
    pub fn op_callvalue(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::CALLVALUE)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let callvalue = self.current_call_frame.msg_value;

        if let Err(err) = self.current_call_frame.stack.push1(callvalue) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // CALLDATALOAD operation
    pub fn op_calldataload(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::CALLDATALOAD)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let calldata_size: U256 = self.current_call_frame.calldata.len().into();

        let offset = match self.current_call_frame.stack.pop1() {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        // If the offset is larger than the actual calldata, then you
        // have no data to return.
        if offset > calldata_size {
            if let Err(err) = self.current_call_frame.stack.push_zero() {
                error.set(err.into());
                return OpcodeResult::Halt;
            };
            return OpcodeResult::Continue;
        };
        let offset: usize = match offset.try_into().map_err(|_| InternalError::TypeConversion) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

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

        if let Err(err) = self.current_call_frame.stack.push1(result) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // CALLDATASIZE operation
    pub fn op_calldatasize(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::CALLDATASIZE)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self
            .current_call_frame
            .stack
            .push1(U256::from(self.current_call_frame.calldata.len()))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // CALLDATACOPY operation
    pub fn op_calldatacopy(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let [dest_offset, calldata_offset, size] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let (size, dest_offset) = match size_offset_to_usize(size, dest_offset) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let calldata_offset = u256_to_usize(calldata_offset).unwrap_or(usize::MAX);

        let new_memory_size = match calculate_memory_size(dest_offset, size) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) =
            gas_cost::calldatacopy(new_memory_size, self.current_call_frame.memory.len(), size)
                .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if size == 0 {
            return OpcodeResult::Continue;
        }

        let calldata_len = self.current_call_frame.calldata.len();

        // offset is out of bounds, so fill zeroes
        if calldata_offset >= calldata_len {
            if let Err(err) = self
                .current_call_frame
                .memory
                .store_zeros(dest_offset, size)
            {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
            return OpcodeResult::Continue;
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
                if let Err(err) = self
                    .current_call_frame
                    .memory
                    .store_data(dest_offset, src_slice)
                {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                };
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

                if let Err(err) = self
                    .current_call_frame
                    .memory
                    .store_data(dest_offset, &data)
                {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            }

            OpcodeResult::Continue
        }
    }

    // CODESIZE operation
    pub fn op_codesize(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::CODESIZE)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self
            .current_call_frame
            .stack
            .push1(U256::from(self.current_call_frame.bytecode.len()))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // CODECOPY operation
    pub fn op_codecopy(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let [dest_offset, code_offset, size] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let (size, dest_offset) = match size_offset_to_usize(size, dest_offset) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let code_offset = u256_to_usize(code_offset).unwrap_or(usize::MAX);

        let new_memory_size = match calculate_memory_size(dest_offset, size) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) =
            gas_cost::codecopy(new_memory_size, self.current_call_frame.memory.len(), size)
                .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if size == 0 {
            return OpcodeResult::Continue;
        }

        // Happiest fast path, copy without an intermediate buffer because there is no need to pad 0s and also size doesn't overflow.
        if let Some(code_offset_end) = code_offset.checked_add(size) {
            if code_offset_end <= self.current_call_frame.bytecode.len() {
                #[expect(unsafe_code, reason = "bounds checked beforehand")]
                let slice = unsafe {
                    self.current_call_frame
                        .bytecode
                        .get_unchecked(code_offset..code_offset_end)
                };
                if let Err(err) = self
                    .current_call_frame
                    .memory
                    .store_data(dest_offset, slice)
                {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }

                return OpcodeResult::Continue;
            }
        }

        let mut data = vec![0u8; size];
        if code_offset < self.current_call_frame.bytecode.len() {
            let diff = self
                .current_call_frame
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
                        .get_unchecked(code_offset..end),
                );
            }
        }

        if let Err(err) = self
            .current_call_frame
            .memory
            .store_data(dest_offset, &data)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // GASPRICE operation
    pub fn op_gasprice(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let gas_price = self.env.gas_price;
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::GASPRICE)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self.current_call_frame.stack.push1(gas_price) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // EXTCODESIZE operation
    pub fn op_extcodesize(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let address = match self.current_call_frame.stack.pop1().map(word_to_address) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let address_was_cold = !self.substate.add_accessed_address(address);
        let account_code_length = match self.db.get_account_code(address) {
            Ok(x) => x.len().into(),
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = gas_cost::extcodesize(address_was_cold)
            .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self.current_call_frame.stack.push1(account_code_length) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // EXTCODECOPY operation
    pub fn op_extcodecopy(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let [address, dest_offset, offset, size] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let address = word_to_address(address);
        let (size, dest_offset) = match size_offset_to_usize(size, dest_offset) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let offset = u256_to_usize(offset).unwrap_or(usize::MAX);

        let current_memory_size = self.current_call_frame.memory.len();
        let address_was_cold = !self.substate.add_accessed_address(address);
        let new_memory_size = match calculate_memory_size(dest_offset, size) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) =
            gas_cost::extcodecopy(size, new_memory_size, current_memory_size, address_was_cold)
                .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if size == 0 {
            return OpcodeResult::Continue;
        }

        // If the bytecode is a delegation designation, it will copy the marker (0xef0100) || address.
        // https://eips.ethereum.org/EIPS/eip-7702#delegation-designation
        let bytecode = match self.db.get_account_code(address) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        // Happiest fast path, copy without an intermediate buffer because there is no need to pad 0s and also size doesn't overflow.
        if let Some(offset_end) = offset.checked_add(size) {
            if offset_end <= bytecode.len() {
                #[expect(unsafe_code, reason = "bounds checked beforehand")]
                let slice = unsafe { bytecode.get_unchecked(offset..offset_end) };
                if let Err(err) = self
                    .current_call_frame
                    .memory
                    .store_data(dest_offset, slice)
                {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }

                return OpcodeResult::Continue;
            }
        }

        let mut data = vec![0u8; size];
        if offset < bytecode.len() {
            let diff = bytecode.len().wrapping_sub(offset);
            let final_size = size.min(diff);
            let end = offset.wrapping_add(final_size);

            #[expect(unsafe_code, reason = "bounds checked beforehand")]
            unsafe {
                data.get_unchecked_mut(..final_size)
                    .copy_from_slice(bytecode.get_unchecked(offset..end));
            }
        }

        if let Err(err) = self
            .current_call_frame
            .memory
            .store_data(dest_offset, &data)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // RETURNDATASIZE operation
    pub fn op_returndatasize(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::RETURNDATASIZE)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self
            .current_call_frame
            .stack
            .push1(U256::from(self.current_call_frame.sub_return_data.len()))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // RETURNDATACOPY operation
    pub fn op_returndatacopy(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let [dest_offset, returndata_offset, size] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let (size, dest_offset) = match size_offset_to_usize(size, dest_offset) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let returndata_offset =
            match u256_to_usize(returndata_offset).map_err(|_| ExceptionalHalt::OutOfBounds) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };

        let new_memory_size = match calculate_memory_size(dest_offset, size) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) =
            gas_cost::returndatacopy(new_memory_size, self.current_call_frame.memory.len(), size)
                .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if size == 0 && returndata_offset == 0 {
            return OpcodeResult::Continue;
        }

        let sub_return_data_len = self.current_call_frame.sub_return_data.len();

        let copy_limit = match returndata_offset
            .checked_add(size)
            .ok_or(ExceptionalHalt::VeryLargeNumber)
        {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if copy_limit > sub_return_data_len {
            error.set(ExceptionalHalt::OutOfBounds.into());
            return OpcodeResult::Halt;
        }

        #[expect(unsafe_code, reason = "bounds checked beforehand")]
        let slice = unsafe {
            self.current_call_frame
                .sub_return_data
                .get_unchecked(returndata_offset..copy_limit)
        };
        if let Err(err) = self
            .current_call_frame
            .memory
            .store_data(dest_offset, slice)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // EXTCODEHASH operation
    pub fn op_extcodehash(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let address = match self.current_call_frame.stack.pop1().map(word_to_address) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let address_was_cold = !self.substate.add_accessed_address(address);
        let account = match self.db.get_account(address) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let account_is_empty = account.is_empty();
        let account_code_hash = account.info.code_hash.0;

        if let Err(err) = gas_cost::extcodehash(address_was_cold)
            .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        // An account is considered empty when it has no code and zero nonce and zero balance. [EIP-161]
        if account_is_empty {
            if let Err(err) = self.current_call_frame.stack.push_zero() {
                error.set(err.into());
                return OpcodeResult::Halt;
            };
            return OpcodeResult::Continue;
        }

        let hash = u256_from_big_endian_const(account_code_hash);
        if let Err(err) = self.current_call_frame.stack.push1(hash) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }
}
