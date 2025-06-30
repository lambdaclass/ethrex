use crate::{
    call_frame::CallFrame,
    constants::{WORD_SIZE, WORD_SIZE_IN_BYTES_USIZE},
    errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError},
    gas_cost::{self, SSTORE_STIPEND},
    memory::{self, calculate_memory_size},
    opcodes::Opcode,
    vm::VM,
};
use ethrex_common::{H256, U256, types::Fork};

// Stack, Memory, Storage and Flow Operations (15)
// Opcodes: POP, MLOAD, MSTORE, MSTORE8, SLOAD, SSTORE, JUMP, JUMPI, PC, MSIZE, GAS, JUMPDEST, TLOAD, TSTORE, MCOPY

impl<'a> VM<'a> {
    // POP operation
    pub fn op_pop(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::POP)?;
        self.stack.pop::<1>()?;
        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // TLOAD operation
    pub fn op_tload(&mut self) -> Result<OpcodeResult, VMError> {
        // [EIP-1153] - TLOAD is only available from CANCUN
        if self.env.config.fork < Fork::Cancun {
            return Err(ExceptionalHalt::InvalidOpcode.into());
        }

        let [key] = *self.stack.pop()?;
        let to = self.current_call_frame()?.to;
        let value = self
            .substate
            .transient_storage
            .get(&(to, key))
            .cloned()
            .unwrap_or(U256::zero());

        let current_call_frame = self.current_call_frame_mut()?;

        current_call_frame.increase_consumed_gas(gas_cost::TLOAD)?;

        self.stack.push(&[value])?;
        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // TSTORE operation
    pub fn op_tstore(&mut self) -> Result<OpcodeResult, VMError> {
        // [EIP-1153] - TLOAD is only available from CANCUN
        if self.env.config.fork < Fork::Cancun {
            return Err(ExceptionalHalt::InvalidOpcode.into());
        }
        let (key, value, to) = {
            let to = {
                let current_call_frame = self.current_call_frame_mut()?;

                current_call_frame.increase_consumed_gas(gas_cost::TSTORE)?;

                if current_call_frame.is_static {
                    return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
                }

                current_call_frame.to
            };

            let [key, value] = *self.stack.pop()?;
            (key, value, to)
        };
        self.substate.transient_storage.insert((to, key), value);

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // MLOAD operation
    pub fn op_mload(&mut self) -> Result<OpcodeResult, VMError> {
        let [offset] = *self.stack.pop()?;

        let word = {
            let current_call_frame = self.current_call_frame_mut()?;

            let new_memory_size = calculate_memory_size(offset, WORD_SIZE_IN_BYTES_USIZE)?;

            current_call_frame.increase_consumed_gas(gas_cost::mload(
                new_memory_size,
                current_call_frame.memory.len(),
            )?)?;

            memory::load_word(&mut current_call_frame.memory, offset)?
        };

        self.stack.push(&[word])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // MSTORE operation
    pub fn op_mstore(&mut self) -> Result<OpcodeResult, VMError> {
        let [offset, value] = *self.stack.pop()?;

        // This is only for debugging purposes of special solidity contracts that enable printing text on screen.
        if self.debug_mode.enabled && self.debug_mode.handle_debug(offset, value)? {
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let current_call_frame = self.current_call_frame_mut()?;

        let new_memory_size = calculate_memory_size(offset, WORD_SIZE_IN_BYTES_USIZE)?;

        current_call_frame.increase_consumed_gas(gas_cost::mstore(
            new_memory_size,
            current_call_frame.memory.len(),
        )?)?;

        memory::try_store_data(
            &mut current_call_frame.memory,
            offset,
            &value.to_big_endian(),
        )?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // MSTORE8 operation
    pub fn op_mstore8(&mut self) -> Result<OpcodeResult, VMError> {
        let [offset] = *self.stack.pop()?;

        let new_memory_size = calculate_memory_size(offset, 1)?;

        {
            let current_call_frame = self.current_call_frame_mut()?;
            current_call_frame.increase_consumed_gas(gas_cost::mstore8(
                new_memory_size,
                current_call_frame.memory.len(),
            )?)?;
        }

        let [value] = *self.stack.pop()?;

        memory::try_store_data(
            &mut self.current_call_frame_mut()?.memory,
            offset,
            &value.to_big_endian()[WORD_SIZE - 1..WORD_SIZE],
        )?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SLOAD operation
    pub fn op_sload(&mut self) -> Result<OpcodeResult, VMError> {
        let (storage_slot_key, address) = {
            let [storage_slot_key] = *self.stack.pop()?;
            let current_call_frame = self.current_call_frame_mut()?;
            let address = current_call_frame.to;
            (storage_slot_key, address)
        };

        let storage_slot_key = H256::from(storage_slot_key.to_big_endian());

        let (value, storage_slot_was_cold) = self.access_storage_slot(address, storage_slot_key)?;

        let current_call_frame = self.current_call_frame_mut()?;

        current_call_frame.increase_consumed_gas(gas_cost::sload(storage_slot_was_cold)?)?;

        self.stack.push(&[value])?;
        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SSTORE operation
    pub fn op_sstore(&mut self) -> Result<OpcodeResult, VMError> {
        if self.current_call_frame()?.is_static {
            return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
        }

        let (storage_slot_key, new_storage_slot_value, to) = {
            let [storage_slot_key, new_storage_slot_value] = *self.stack.pop()?;
            let current_call_frame = self.current_call_frame_mut()?;
            let to = current_call_frame.to;
            (storage_slot_key, new_storage_slot_value, to)
        };

        // EIP-2200
        let gas_left = self.current_call_frame()?.gas_remaining;
        if gas_left <= SSTORE_STIPEND {
            return Err(ExceptionalHalt::OutOfGas.into());
        }

        // Get current and original (pre-tx) values.
        let key = H256::from(storage_slot_key.to_big_endian());
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

        self.current_call_frame_mut()?
            .increase_consumed_gas(gas_cost::sstore(
                original_value,
                current_value,
                new_storage_slot_value,
                storage_slot_was_cold,
            )?)?;

        self.update_account_storage(to, key, new_storage_slot_value)?;
        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // MSIZE operation
    pub fn op_msize(&mut self) -> Result<OpcodeResult, VMError> {
        let size = {
            let current_call_frame = self.current_call_frame_mut()?;
            current_call_frame.increase_consumed_gas(gas_cost::MSIZE)?;
            current_call_frame.memory.len().into()
        };
        self.stack.push(&[size])?;
        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // GAS operation
    pub fn op_gas(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::GAS)?;

        let remaining_gas = current_call_frame.gas_remaining;
        // Note: These are not consumed gas calculations, but are related, so I used this wrapping here
        self.stack.push(&[remaining_gas.into()])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // MCOPY operation
    pub fn op_mcopy(&mut self) -> Result<OpcodeResult, VMError> {
        // [EIP-5656] - MCOPY is only available from CANCUN
        if self.env.config.fork < Fork::Cancun {
            return Err(ExceptionalHalt::InvalidOpcode.into());
        }

        let [dest_offset, src_offset, size] = *self.stack.pop()?;
        let current_call_frame = self.current_call_frame_mut()?;
        let size: usize = size
            .try_into()
            .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;

        let new_memory_size_for_dest = calculate_memory_size(dest_offset, size)?;

        let new_memory_size_for_src = calculate_memory_size(src_offset, size)?;

        current_call_frame.increase_consumed_gas(gas_cost::mcopy(
            new_memory_size_for_dest.max(new_memory_size_for_src),
            current_call_frame.memory.len(),
            size,
        )?)?;

        memory::try_copy_within(
            &mut current_call_frame.memory,
            src_offset,
            dest_offset,
            size,
        )?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // JUMP operation
    pub fn op_jump(&mut self) -> Result<OpcodeResult, VMError> {
        {
            let current_call_frame = self.current_call_frame_mut()?;
            current_call_frame.increase_consumed_gas(gas_cost::JUMP)?;
        }

        let [jump_address] = *self.stack.pop()?;
        Self::jump(self.current_call_frame_mut()?, jump_address)?;

        Ok(OpcodeResult::Continue { pc_increment: 0 })
    }

    /// Check if the jump destination is valid by:
    ///   - Checking that the byte at the requested target PC is a JUMPDEST (0x5B).
    ///   - Ensuring the byte is not blacklisted. In other words, the 0x5B value is not part of a
    ///     constant associated with a push instruction.
    fn target_address_is_valid(call_frame: &CallFrame, jump_address: usize) -> bool {
        #[expect(clippy::as_conversions)]
        call_frame.bytecode.get(jump_address).is_some_and(|&value| {
            // It's a constant, therefore the conversion cannot fail.
            value == Opcode::JUMPDEST as u8
                && call_frame
                    .invalid_jump_destinations
                    .binary_search(&jump_address)
                    .is_err()
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

        if Self::target_address_is_valid(call_frame, jump_address_usize) {
            call_frame.pc = jump_address_usize;
            Ok(())
        } else {
            Err(ExceptionalHalt::InvalidJump.into())
        }
    }

    // JUMPI operation
    pub fn op_jumpi(&mut self) -> Result<OpcodeResult, VMError> {
        let [jump_address, condition] = *self.stack.pop()?;
        let current_call_frame = self.current_call_frame_mut()?;

        current_call_frame.increase_consumed_gas(gas_cost::JUMPI)?;

        let pc_increment = if !condition.is_zero() {
            // Move the PC but don't increment it afterwards
            Self::jump(current_call_frame, jump_address)?;
            0
        } else {
            1
        };
        Ok(OpcodeResult::Continue { pc_increment })
    }

    // JUMPDEST operation
    pub fn op_jumpdest(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::JUMPDEST)?;
        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // PC operation
    pub fn op_pc(&mut self) -> Result<OpcodeResult, VMError> {
        let pc = {
            let current_call_frame = self.current_call_frame_mut()?;
            current_call_frame.increase_consumed_gas(gas_cost::PC)?;
            current_call_frame.pc
        };

        self.stack.push(&[U256::from(pc)])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
