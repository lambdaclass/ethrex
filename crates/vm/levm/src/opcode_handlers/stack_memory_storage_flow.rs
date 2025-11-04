use crate::{
    call_frame::CallFrame,
    constants::{WORD_SIZE, WORD_SIZE_IN_BYTES_USIZE},
    errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError},
    gas_cost::{self, SSTORE_STIPEND},
    memory::calculate_memory_size,
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
    pub fn op_pop(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::POP)?;
        self.current_stack().pop1()?;
        Ok(OpcodeResult::Continue)
    }

    // TLOAD operation
    pub fn op_tload(&mut self) -> Result<OpcodeResult, VMError> {
        let key = self.current_stack().pop1()?;
        let to = self.current_call_frame.to;
        let value = self.substate.get_transient(&to, &key);

        self.current_call_frame
            .increase_consumed_gas(gas_cost::TLOAD)?;

        self.current_stack().push1(value)?;
        Ok(OpcodeResult::Continue)
    }

    // TSTORE operation
    pub fn op_tstore(&mut self) -> Result<OpcodeResult, VMError> {
        let (key, value, to) = {
            self.current_call_frame
                .increase_consumed_gas(gas_cost::TSTORE)?;

            if self.current_call_frame.is_static {
                return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
            }

            let [key, value] = *self.current_stack().pop()?;
            (key, value, self.current_call_frame.to)
        };
        self.substate.set_transient(&to, &key, value);

        Ok(OpcodeResult::Continue)
    }

    // MLOAD operation
    pub fn op_mload(&mut self) -> Result<OpcodeResult, VMError> {
        let offset = u256_to_usize(self.current_stack().pop1()?)?;

        let new_memory_size = calculate_memory_size(offset, WORD_SIZE_IN_BYTES_USIZE)?;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::mload(
                new_memory_size,
                self.current_call_frame.memory.len(),
            )?)?;

        let word = self.current_call_frame.memory.load_word(offset)?;

        self.current_stack().push1(word)?;

        Ok(OpcodeResult::Continue)
    }

    // MSTORE operation
    pub fn op_mstore(&mut self) -> Result<OpcodeResult, VMError> {
        let [offset, value] = *self.current_stack().pop()?;

        // This is only for debugging purposes of special solidity contracts that enable printing text on screen.
        if self.debug_mode.enabled && self.debug_mode.handle_debug(offset, value)? {
            return Ok(OpcodeResult::Continue);
        }

        let offset = u256_to_usize(offset)?;

        let new_memory_size = calculate_memory_size(offset, WORD_SIZE_IN_BYTES_USIZE)?;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::mstore(
                new_memory_size,
                self.current_call_frame.memory.len(),
            )?)?;

        self.current_call_frame.memory.store_word(offset, value)?;

        Ok(OpcodeResult::Continue)
    }

    // MSTORE8 operation
    pub fn op_mstore8(&mut self) -> Result<OpcodeResult, VMError> {
        let offset = u256_to_usize(self.current_stack().pop1()?)?;

        let new_memory_size = calculate_memory_size(offset, 1)?;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::mstore8(
                new_memory_size,
                self.current_call_frame.memory.len(),
            )?)?;

        let value = self.current_stack().pop1()?;

        self.current_call_frame
            .memory
            .store_data(offset, &u256_to_big_endian(value)[WORD_SIZE - 1..WORD_SIZE])?;

        Ok(OpcodeResult::Continue)
    }

    // SLOAD operation
    pub fn op_sload(&mut self) -> Result<OpcodeResult, VMError> {
        let (storage_slot_key, address) = {
            let storage_slot_key = self.current_stack().pop1()?;
            let address = self.current_call_frame.to;
            (storage_slot_key, address)
        };

        let storage_slot_key = u256_to_h256(storage_slot_key);

        let (value, storage_slot_was_cold) = self.access_storage_slot(address, storage_slot_key)?;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::sload(storage_slot_was_cold)?)?;

        self.current_stack().push1(value)?;
        Ok(OpcodeResult::Continue)
    }

    // SSTORE operation
    pub fn op_sstore(&mut self) -> Result<OpcodeResult, VMError> {
        if self.current_call_frame.is_static {
            return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
        }

        let (storage_slot_key, new_storage_slot_value, to) = {
            let [storage_slot_key, new_storage_slot_value] = *self.current_stack().pop()?;
            let to = self.current_call_frame.to;
            (storage_slot_key, new_storage_slot_value, to)
        };

        // EIP-2200
        let gas_left = self.current_call_frame.gas_remaining;
        if gas_left <= SSTORE_STIPEND {
            return Err(ExceptionalHalt::OutOfGas.into());
        }

        // Get current and original (pre-tx) values.
        let key = u256_to_h256(storage_slot_key);
        let (current_value, storage_slot_was_cold) = self.access_storage_slot(to, key)?;
        let original_value = self.get_original_storage(to, key)?;

        // Gas Refunds
        // Sync gas refund with global env, ensuring consistency accross contexts.
        let mut gas_refunds = self.substate.refunded_gas;

        // https://eips.ethereum.org/EIPS/eip-2929
        let (remove_slot_cost, restore_empty_slot_cost, restore_slot_cost) = (4800, 19900, 2800);

        if new_storage_slot_value != current_value {
            if current_value == original_value {
                if !original_value.is_zero() && new_storage_slot_value.is_zero() {
                    gas_refunds = gas_refunds
                        .checked_add(remove_slot_cost)
                        .ok_or(InternalError::Overflow)?;
                }
            } else {
                if original_value != U256::zero() {
                    if current_value == U256::zero() {
                        gas_refunds = gas_refunds
                            .checked_sub(remove_slot_cost)
                            .ok_or(InternalError::Underflow)?;
                    } else if new_storage_slot_value.is_zero() {
                        gas_refunds = gas_refunds
                            .checked_add(remove_slot_cost)
                            .ok_or(InternalError::Overflow)?;
                    }
                }
                if new_storage_slot_value == original_value {
                    if original_value == U256::zero() {
                        gas_refunds = gas_refunds
                            .checked_add(restore_empty_slot_cost)
                            .ok_or(InternalError::Overflow)?;
                    } else {
                        gas_refunds = gas_refunds
                            .checked_add(restore_slot_cost)
                            .ok_or(InternalError::Overflow)?;
                    }
                }
            }
        }

        self.substate.refunded_gas = gas_refunds;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::sstore(
                original_value,
                current_value,
                new_storage_slot_value,
                storage_slot_was_cold,
            )?)?;

        if new_storage_slot_value != current_value {
            self.update_account_storage(to, key, new_storage_slot_value, current_value)?;
        }

        Ok(OpcodeResult::Continue)
    }

    // MSIZE operation
    pub fn op_msize(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::MSIZE)?;
        let mem_size = self.current_call_frame.memory.len().into();
        self.current_stack().push1(mem_size)?;
        Ok(OpcodeResult::Continue)
    }

    // GAS operation
    pub fn op_gas(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::GAS)?;

        let remaining_gas = self.current_call_frame.gas_remaining;
        // Note: These are not consumed gas calculations, but are related, so I used this wrapping here
        self.current_stack().push1(remaining_gas.into())?;

        Ok(OpcodeResult::Continue)
    }

    // MCOPY operation
    pub fn op_mcopy(&mut self) -> Result<OpcodeResult, VMError> {
        let [dest_offset, src_offset, size] = *self.current_stack().pop()?;
        let size: usize = u256_to_usize(size)?;

        let (dest_offset, src_offset) = if size == 0 {
            (0, 0)
        } else {
            (u256_to_usize(dest_offset)?, u256_to_usize(src_offset)?)
        };

        let new_memory_size = calculate_memory_size(dest_offset.max(src_offset), size)?;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::mcopy(
                new_memory_size,
                self.current_call_frame.memory.len(),
                size,
            )?)?;

        self.current_call_frame
            .memory
            .copy_within(src_offset, dest_offset, size)?;

        Ok(OpcodeResult::Continue)
    }

    // JUMP operation
    pub fn op_jump(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::JUMP)?;

        let jump_address = self.current_stack().pop1()?;
        Self::jump(&mut self.current_call_frame, jump_address)?;

        Ok(OpcodeResult::Continue)
    }

    /// Check if the jump destination is valid by:
    ///   - Checking that the byte at the requested target PC is a JUMPDEST (0x5B).
    ///   - Ensuring the byte is not blacklisted. In other words, the 0x5B value is not part of a
    ///     constant associated with a push instruction.
    fn target_address_is_valid(call_frame: &mut CallFrame, jump_address: u16) -> bool {
        call_frame
            .bytecode
            .jump_targets
            .binary_search(&jump_address)
            .is_ok()
    }

    /// JUMP* family (`JUMP` and `JUMP` ATTOW [DEC 2024]) helper
    /// function.
    /// This function will change the PC for the specified call frame
    /// to be equal to the specified address. If the address is not a
    /// valid JUMPDEST, it will return an error
    pub fn jump(call_frame: &mut CallFrame, jump_address: U256) -> Result<(), VMError> {
        let jump_address_u16 = jump_address
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

        #[expect(clippy::arithmetic_side_effects)]
        if Self::target_address_is_valid(call_frame, jump_address_u16) {
            call_frame.increase_consumed_gas(gas_cost::JUMPDEST)?;
            call_frame.pc = usize::from(jump_address_u16) + 1;
            Ok(())
        } else {
            Err(ExceptionalHalt::InvalidJump.into())
        }
    }

    // JUMPI operation
    pub fn op_jumpi(&mut self) -> Result<OpcodeResult, VMError> {
        let [jump_address, condition] = *self.current_stack().pop()?;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::JUMPI)?;

        if !condition.is_zero() {
            // Move the PC but don't increment it afterwards
            Self::jump(&mut self.current_call_frame, jump_address)?;
        }

        Ok(OpcodeResult::Continue)
    }

    // JUMPDEST operation
    pub fn op_jumpdest(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::JUMPDEST)?;

        Ok(OpcodeResult::Continue)
    }

    // PC operation
    pub fn op_pc(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::PC)?;

        let pc = self.current_call_frame.pc.wrapping_sub(1);
        self.current_stack().push1(U256::from(pc))?;

        Ok(OpcodeResult::Continue)
    }
}
