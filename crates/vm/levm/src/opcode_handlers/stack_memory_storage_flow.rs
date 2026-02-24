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
    #[inline]
    pub fn op_pop(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.stack.pop1()?;
        Ok(OpcodeResult::Continue)
    }

    // TLOAD operation
    pub fn op_tload(&mut self) -> Result<OpcodeResult, VMError> {
        let key = self.current_call_frame.stack.pop1()?;
        let to = self.current_call_frame.to;
        let value = self.substate.get_transient(&to, &key);

        let current_call_frame = &mut self.current_call_frame;

        current_call_frame.stack.push(value)?;
        Ok(OpcodeResult::Continue)
    }

    // TSTORE operation
    pub fn op_tstore(&mut self) -> Result<OpcodeResult, VMError> {
        let (key, value, to) = {
            let current_call_frame = &mut self.current_call_frame;

            if current_call_frame.is_static {
                return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
            }

            let [key, value] = *current_call_frame.stack.pop()?;
            (key, value, current_call_frame.to)
        };
        self.substate.set_transient(&to, &key, value);

        Ok(OpcodeResult::Continue)
    }

    // MLOAD operation
    #[inline]
    pub fn op_mload(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        let offset = u256_to_usize(current_call_frame.stack.pop1()?)?;

        let new_memory_size = calculate_memory_size(offset, WORD_SIZE_IN_BYTES_USIZE)?;

        current_call_frame.increase_consumed_gas(gas_cost::mload(
            new_memory_size,
            current_call_frame.memory.len(),
        )?)?;

        current_call_frame
            .stack
            .push(current_call_frame.memory.load_word(offset)?)?;

        Ok(OpcodeResult::Continue)
    }

    // MSTORE operation
    #[inline]
    pub fn op_mstore(&mut self) -> Result<OpcodeResult, VMError> {
        let [offset, value] = *self.current_call_frame.stack.pop()?;

        // This is only for debugging purposes of special solidity contracts that enable printing text on screen.
        if self.debug_mode.enabled && self.debug_mode.handle_debug(offset, value)? {
            return Ok(OpcodeResult::Continue);
        }

        let offset = u256_to_usize(offset)?;

        let current_call_frame = &mut self.current_call_frame;

        let new_memory_size = calculate_memory_size(offset, WORD_SIZE_IN_BYTES_USIZE)?;

        current_call_frame.increase_consumed_gas(gas_cost::mstore(
            new_memory_size,
            current_call_frame.memory.len(),
        )?)?;

        current_call_frame.memory.store_word(offset, value)?;

        Ok(OpcodeResult::Continue)
    }

    // MSTORE8 operation
    pub fn op_mstore8(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;

        let offset = u256_to_usize(current_call_frame.stack.pop1()?)?;

        let new_memory_size = calculate_memory_size(offset, 1)?;

        current_call_frame.increase_consumed_gas(gas_cost::mstore8(
            new_memory_size,
            current_call_frame.memory.len(),
        )?)?;

        let value = current_call_frame.stack.pop1()?;

        current_call_frame
            .memory
            .store_data(offset, &u256_to_big_endian(value)[WORD_SIZE - 1..WORD_SIZE])?;

        Ok(OpcodeResult::Continue)
    }

    // SLOAD operation
    #[inline]
    pub fn op_sload(&mut self) -> Result<OpcodeResult, VMError> {
        let (storage_slot_key, address) = {
            let current_call_frame = &mut self.current_call_frame;
            let storage_slot_key = current_call_frame.stack.pop1()?;
            let address = current_call_frame.to;
            (storage_slot_key, address)
        };

        let key = u256_to_h256(storage_slot_key);

        // Note: access_storage_slot does NOT record to BAL per EIP-7928.
        // BAL recording must happen AFTER gas check passes.
        let (value, storage_slot_was_cold) = self.access_storage_slot(address, key)?;

        self.current_call_frame
            .increase_consumed_gas(gas_cost::sload(storage_slot_was_cold)?)?;

        // Record to BAL AFTER gas check passes per EIP-7928:
        // "If pre-state validation fails, the target is never accessed and must not appear in BAL."
        self.record_storage_slot_to_bal(address, storage_slot_key);

        self.current_call_frame.stack.push(value)?;
        Ok(OpcodeResult::Continue)
    }

    // SSTORE operation
    pub fn op_sstore(&mut self) -> Result<OpcodeResult, VMError> {
        if self.current_call_frame.is_static {
            return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
        }

        let (storage_slot_key, new_storage_slot_value, to) = {
            let current_call_frame = &mut self.current_call_frame;
            let [storage_slot_key, new_storage_slot_value] = *current_call_frame.stack.pop()?;
            let to = current_call_frame.to;
            (storage_slot_key, new_storage_slot_value, to)
        };

        // EIP-2200: SSTORE_STIPEND check BEFORE accessing storage
        // Per EIP-7928: "If SSTORE fails the GAS_CALL_STIPEND check, the storage slot MUST NOT
        // appear in storage_reads or storage_changes."
        let gas_left = self.current_call_frame.gas_remaining;
        if gas_left <= SSTORE_STIPEND {
            return Err(ExceptionalHalt::OutOfGas.into());
        }

        // Get current and original (pre-tx) values.
        // Note: access_storage_slot does NOT record to BAL per EIP-7928.
        // BAL recording happens AFTER the SSTORE_STIPEND check but BEFORE the main gas check.
        let key = u256_to_h256(storage_slot_key);
        let (current_value, storage_slot_was_cold) = self.access_storage_slot(to, key)?;
        let original_value = self.get_original_storage(to, key)?;

        // Record storage read to BAL AFTER SSTORE_STIPEND check passes, BEFORE main gas check.
        // Per EIP-7928 test_bal_sstore_and_oog: if SSTORE passes the stipend check but fails the
        // main gas charge (charge_gas), the slot MUST appear as a read because the implicit SLOAD
        // has already happened (to get current_value and original_value).
        // "If SSTORE fails the GAS_CALL_STIPEND check, the storage slot MUST NOT appear in BAL"
        // only refers to the stipend check, NOT the main gas check.
        self.record_storage_slot_to_bal(to, storage_slot_key);

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

        // Main gas check - if this fails, the storage read is already recorded above
        self.current_call_frame
            .increase_consumed_gas(gas_cost::sstore(
                original_value,
                current_value,
                new_storage_slot_value,
                storage_slot_was_cold,
            )?)?;

        // Note: BAL read already recorded above (after stipend check, before gas check).
        // If value changes, update_account_storage will record the write.

        if new_storage_slot_value != current_value {
            self.update_account_storage(
                to,
                key,
                storage_slot_key,
                new_storage_slot_value,
                current_value,
            )?;
        }

        Ok(OpcodeResult::Continue)
    }

    // MSIZE operation
    pub fn op_msize(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame
            .stack
            .push(current_call_frame.memory.len().into())?;
        Ok(OpcodeResult::Continue)
    }

    // GAS operation
    pub fn op_gas(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        let remaining_gas = current_call_frame.gas_remaining;
        // Note: These are not consumed gas calculations, but are related, so I used this wrapping here
        current_call_frame.stack.push(remaining_gas.into())?;

        // Pre-charge the static gas cost of the next basic block.
        // GAS is a block terminator so that it reports the correct
        // gas_remaining (only its own cost deducted, not future opcodes).
        // Skip if the next instruction is a JUMPDEST (op_jumpdest will
        // pre-charge its own block to avoid double-charging).
        if current_call_frame
            .bytecode
            .bytecode
            .get(current_call_frame.pc)
            != Some(&0x5B)
        {
            current_call_frame.increase_consumed_gas(
                current_call_frame
                    .bytecode
                    .block_cost(current_call_frame.pc as u32),
            )?;
        }

        Ok(OpcodeResult::Continue)
    }

    // MCOPY operation
    pub fn op_mcopy(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        let [dest_offset, src_offset, size] = *current_call_frame.stack.pop()?;
        let size: usize = u256_to_usize(size)?;

        let (dest_offset, src_offset) = if size == 0 {
            (0, 0)
        } else {
            (u256_to_usize(dest_offset)?, u256_to_usize(src_offset)?)
        };

        let new_memory_size = calculate_memory_size(dest_offset.max(src_offset), size)?;

        current_call_frame.increase_consumed_gas(gas_cost::mcopy(
            new_memory_size,
            current_call_frame.memory.len(),
            size,
        )?)?;

        current_call_frame
            .memory
            .copy_within(src_offset, dest_offset, size)?;

        Ok(OpcodeResult::Continue)
    }

    // JUMP operation
    pub fn op_jump(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;

        let jump_address = current_call_frame.stack.pop1()?;
        Self::jump(current_call_frame, jump_address)?;

        Ok(OpcodeResult::Continue)
    }

    /// Check if the jump destination is valid by:
    ///   - Checking that the byte at the requested target PC is a JUMPDEST (0x5B).
    ///   - Ensuring the byte is not blacklisted. In other words, the 0x5B value is not part of a
    ///     constant associated with a push instruction.
    fn target_address_is_valid(call_frame: &CallFrame, jump_address: u32) -> bool {
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
        let jump_address_u32 = jump_address
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

        #[expect(clippy::arithmetic_side_effects)]
        if Self::target_address_is_valid(call_frame, jump_address_u32) {
            // Pre-charge the static gas cost of the target basic block.
            call_frame.increase_consumed_gas(call_frame.bytecode.block_cost(jump_address_u32))?;
            call_frame.pc = usize::try_from(jump_address_u32)
                .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?
                + 1;
            Ok(())
        } else {
            Err(ExceptionalHalt::InvalidJump.into())
        }
    }

    // JUMPI operation
    pub fn op_jumpi(&mut self) -> Result<OpcodeResult, VMError> {
        let [jump_address, condition] = *self.current_call_frame.stack.pop()?;

        if !condition.is_zero() {
            // Move the PC but don't increment it afterwards.
            // jump() pre-charges the target block's static gas cost.
            Self::jump(&mut self.current_call_frame, jump_address)?;
        } else {
            // Pre-charge the static gas cost of the fallthrough block,
            // unless the next instruction is a JUMPDEST (op_jumpdest
            // will pre-charge its own block to avoid double-charging).
            let next_pc = self.current_call_frame.pc;
            if self.current_call_frame.bytecode.bytecode.get(next_pc) != Some(&0x5B) {
                self.current_call_frame.increase_consumed_gas(
                    self.current_call_frame.bytecode.block_cost(next_pc as u32),
                )?;
            }
        }

        Ok(OpcodeResult::Continue)
    }

    // JUMPDEST operation
    pub fn op_jumpdest(&mut self) -> Result<OpcodeResult, VMError> {
        // Pre-charge the static gas cost of the JUMPDEST's basic block
        // when reached via sequential fallthrough. When reached via
        // JUMP/JUMPI, jump() already pre-charged and op_jumpdest is
        // never called (pc skips past the JUMPDEST).
        let jumpdest_pc = self.current_call_frame.pc.wrapping_sub(1) as u32;
        self.current_call_frame
            .increase_consumed_gas(self.current_call_frame.bytecode.block_cost(jumpdest_pc))?;
        Ok(OpcodeResult::Continue)
    }

    // PC operation
    pub fn op_pc(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame
            .stack
            .push(U256::from(current_call_frame.pc.wrapping_sub(1)))?;

        Ok(OpcodeResult::Continue)
    }
}
