use std::cell::OnceCell;

use crate::{
    call_frame::CallFrame,
    constants::{WORD_SIZE, WORD_SIZE_IN_BYTES_USIZE},
    errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError},
    gas_cost::{self, SSTORE_STIPEND},
    memory::calculate_memory_size,
    opcodes::Opcode,
    utils::u256_to_usize,
    vm::VM,
};
use ethrex_common::{
    U256,
    utils::{u256_to_big_endian, u256_to_h256},
};

// Stack, Memory, Storage and Flow Operations (15)
// Opcodes: POP, MLOAD, MSTORE, MSTORE8, SLOAD, SSTORE, JUMP, JUMPI, PC, MSIZE, GAS, JUMPDEST, TLOAD, TSTORE, MCOPY

pub const OUT_OF_BOUNDS: U256 = U256([u64::MAX, 0, 0, 0]);

impl<'a> VM<'a> {
    // POP operation
    pub fn op_pop(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::POP) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self.current_call_frame.stack.pop1() {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        OpcodeResult::Continue
    }

    // TLOAD operation
    pub fn op_tload(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let key = match self.current_call_frame.stack.pop1() {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let to = self.current_call_frame.to;
        let value = self.substate.get_transient(&to, &key);

        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::TLOAD)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self.current_call_frame.stack.push1(value) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // TSTORE operation
    pub fn op_tstore(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let (key, value, to) = {
            if let Err(err) = self
                .current_call_frame
                .increase_consumed_gas(gas_cost::TSTORE)
            {
                error.set(err.into());
                return OpcodeResult::Halt;
            }

            if self.current_call_frame.is_static {
                error.set(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
                return OpcodeResult::Halt;
            }

            let [key, value] = match self.current_call_frame.stack.pop() {
                Ok(x) => *x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };

            (key, value, self.current_call_frame.to)
        };
        self.substate.set_transient(&to, &key, value);

        OpcodeResult::Continue
    }

    // MLOAD operation
    pub fn op_mload(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let offset = match self.current_call_frame.stack.pop1().and_then(u256_to_usize) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let new_memory_size = match calculate_memory_size(offset, WORD_SIZE_IN_BYTES_USIZE) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = gas_cost::mload(new_memory_size, self.current_call_frame.memory.len())
            .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self
            .current_call_frame
            .memory
            .load_word(offset)
            .and_then(|x| Ok(self.current_call_frame.stack.push1(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // MSTORE operation
    pub fn op_mstore(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let [offset, value] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        // This is only for debugging purposes of special solidity contracts that enable printing text on screen.
        if self.debug_mode.enabled
            && match self.debug_mode.handle_debug(offset, value) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            }
        {
            return OpcodeResult::Continue;
        }

        let offset = match u256_to_usize(offset) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let new_memory_size = match calculate_memory_size(offset, WORD_SIZE_IN_BYTES_USIZE) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = gas_cost::mstore(new_memory_size, self.current_call_frame.memory.len())
            .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        if let Err(err) = self.current_call_frame.memory.store_word(offset, value) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // MSTORE8 operation
    pub fn op_mstore8(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let offset = match self.current_call_frame.stack.pop1().and_then(u256_to_usize) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let new_memory_size = match calculate_memory_size(offset, 1) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = gas_cost::mstore8(new_memory_size, self.current_call_frame.memory.len())
            .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        let value = match self.current_call_frame.stack.pop1() {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = self
            .current_call_frame
            .memory
            .store_data(offset, &u256_to_big_endian(value)[WORD_SIZE - 1..WORD_SIZE])
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        OpcodeResult::Continue
    }

    // SLOAD operation
    pub fn op_sload(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let (storage_slot_key, address) = {
            let storage_slot_key = match self.current_call_frame.stack.pop1() {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };
            let address = self.current_call_frame.to;
            (storage_slot_key, address)
        };

        let storage_slot_key = u256_to_h256(storage_slot_key);

        let (value, storage_slot_was_cold) =
            match self.access_storage_slot(address, storage_slot_key) {
                Ok(x) => x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };

        if let Err(err) = gas_cost::sload(storage_slot_was_cold)
            .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        if let Err(err) = self.current_call_frame.stack.push1(value) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // SSTORE operation
    pub fn op_sstore(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if self.current_call_frame.is_static {
            error.set(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
            return OpcodeResult::Halt;
        }

        let (storage_slot_key, new_storage_slot_value, to) = {
            let current_call_frame = &mut self.current_call_frame;
            let [storage_slot_key, new_storage_slot_value] = match current_call_frame.stack.pop() {
                Ok(x) => *x,
                Err(err) => {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
            };
            let to = current_call_frame.to;
            (storage_slot_key, new_storage_slot_value, to)
        };

        // EIP-2200
        let gas_left = self.current_call_frame.gas_remaining;
        if gas_left <= SSTORE_STIPEND {
            error.set(ExceptionalHalt::OutOfGas.into());
            return OpcodeResult::Halt;
        }

        // Get current and original (pre-tx) values.
        let key = u256_to_h256(storage_slot_key);
        let (current_value, storage_slot_was_cold) = match self.access_storage_slot(to, key) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let original_value = match self.get_original_storage(to, key) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        // Gas Refunds
        // Sync gas refund with global env, ensuring consistency accross contexts.
        let mut gas_refunds = self.substate.refunded_gas;

        // https://eips.ethereum.org/EIPS/eip-2929
        let (remove_slot_cost, restore_empty_slot_cost, restore_slot_cost) = (4800, 19900, 2800);

        if new_storage_slot_value != current_value {
            if current_value == original_value {
                if !original_value.is_zero() && new_storage_slot_value.is_zero() {
                    gas_refunds = match gas_refunds
                        .checked_add(remove_slot_cost)
                        .ok_or(InternalError::Overflow)
                    {
                        Ok(x) => x,
                        Err(err) => {
                            error.set(err.into());
                            return OpcodeResult::Halt;
                        }
                    };
                }
            } else {
                if original_value != U256::zero() {
                    if current_value == U256::zero() {
                        gas_refunds = match gas_refunds
                            .checked_sub(remove_slot_cost)
                            .ok_or(InternalError::Underflow)
                        {
                            Ok(x) => x,
                            Err(err) => {
                                error.set(err.into());
                                return OpcodeResult::Halt;
                            }
                        };
                    } else if new_storage_slot_value.is_zero() {
                        gas_refunds = match gas_refunds
                            .checked_add(remove_slot_cost)
                            .ok_or(InternalError::Overflow)
                        {
                            Ok(x) => x,
                            Err(err) => {
                                error.set(err.into());
                                return OpcodeResult::Halt;
                            }
                        };
                    }
                }
                if new_storage_slot_value == original_value {
                    if original_value == U256::zero() {
                        gas_refunds = match gas_refunds
                            .checked_add(restore_empty_slot_cost)
                            .ok_or(InternalError::Overflow)
                        {
                            Ok(x) => x,
                            Err(err) => {
                                error.set(err.into());
                                return OpcodeResult::Halt;
                            }
                        };
                    } else {
                        gas_refunds = match gas_refunds
                            .checked_add(restore_slot_cost)
                            .ok_or(InternalError::Overflow)
                        {
                            Ok(x) => x,
                            Err(err) => {
                                error.set(err.into());
                                return OpcodeResult::Halt;
                            }
                        };
                    }
                }
            }
        }

        self.substate.refunded_gas = gas_refunds;

        gas_cost::sstore(
            original_value,
            current_value,
            new_storage_slot_value,
            storage_slot_was_cold,
        )
        .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?));

        if new_storage_slot_value != current_value {
            if let Err(err) =
                self.update_account_storage(to, key, new_storage_slot_value, current_value)
            {
                error.set(err.into());
                return OpcodeResult::Halt;
            };
        }

        OpcodeResult::Continue
    }

    // MSIZE operation
    pub fn op_msize(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::MSIZE)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        if let Err(err) = self
            .current_call_frame
            .stack
            .push1(self.current_call_frame.memory.len().into())
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        OpcodeResult::Continue
    }

    // GAS operation
    pub fn op_gas(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::GAS) {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        let remaining_gas = self.current_call_frame.gas_remaining;
        // Note: These are not consumed gas calculations, but are related, so I used this wrapping here
        if let Err(err) = self.current_call_frame.stack.push1(remaining_gas.into()) {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        OpcodeResult::Continue
    }

    // MCOPY operation
    pub fn op_mcopy(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let [dest_offset, src_offset, size] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let size: usize = match u256_to_usize(size) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let (dest_offset, src_offset) = if size == 0 {
            (0, 0)
        } else {
            (
                match u256_to_usize(dest_offset) {
                    Ok(x) => x,
                    Err(err) => {
                        error.set(err.into());
                        return OpcodeResult::Halt;
                    }
                },
                match u256_to_usize(src_offset) {
                    Ok(x) => x,
                    Err(err) => {
                        error.set(err.into());
                        return OpcodeResult::Halt;
                    }
                },
            )
        };

        let new_memory_size = match calculate_memory_size(dest_offset.max(src_offset), size) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) =
            gas_cost::mcopy(new_memory_size, self.current_call_frame.memory.len(), size)
                .and_then(|x| Ok(self.current_call_frame.increase_consumed_gas(x)?))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self
            .current_call_frame
            .memory
            .copy_within(src_offset, dest_offset, size)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // JUMP operation
    pub fn op_jump(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::JUMP)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let jump_address = match self.current_call_frame.stack.pop1() {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = Self::jump(&mut self.current_call_frame, jump_address) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    /// Check if the jump destination is valid by:
    ///   - Checking that the byte at the requested target PC is a JUMPDEST (0x5B).
    ///   - Ensuring the byte is not blacklisted. In other words, the 0x5B value is not part of a
    ///     constant associated with a push instruction.
    fn target_address_is_valid(call_frame: &mut CallFrame, jump_address: usize) -> bool {
        #[expect(clippy::as_conversions)]
        call_frame.bytecode.get(jump_address).is_some_and(|value| {
            // It's a constant, therefore the conversion cannot fail.
            *value == Opcode::JUMPDEST as u8
                && !call_frame.jump_target_filter.is_blacklisted(jump_address)
        })
    }

    /// JUMP* family (`JUMP` and `JUMP` ATTOW [DEC 2024]) helper
    /// function.
    /// This function will change the PC for the specified call frame
    /// to be equal to the specified address. If the address is not a
    /// valid JUMPDEST, it will return an error
    pub fn jump(call_frame: &mut CallFrame, jump_address: U256) -> Result<(), VMError> {
        let jump_address_usize = jump_address
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

        #[expect(clippy::arithmetic_side_effects)]
        if Self::target_address_is_valid(call_frame, jump_address_usize) {
            call_frame.increase_consumed_gas(gas_cost::JUMPDEST)?;
            call_frame.pc = jump_address_usize + 1;
            Ok(())
        } else {
            Err(ExceptionalHalt::InvalidJump.into())
        }
    }

    // JUMPI operation
    pub fn op_jumpi(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let [jump_address, condition] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::JUMPI)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if !condition.is_zero() {
            // Move the PC but don't increment it afterwards
            if let Err(err) = Self::jump(&mut self.current_call_frame, jump_address) {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        }

        OpcodeResult::Continue
    }

    // JUMPDEST operation
    pub fn op_jumpdest(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::JUMPDEST)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // PC operation
    pub fn op_pc(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::PC) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self
            .current_call_frame
            .stack
            .push1(U256::from(self.current_call_frame.pc.wrapping_sub(1)))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }
}
