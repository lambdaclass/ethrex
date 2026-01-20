use crate::{
    call_frame::CallFrame,
    constants::{WORD_SIZE, WORD_SIZE_IN_BYTES_USIZE},
    errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError},
    gas_cost::{self, SSTORE_STIPEND},
    gas_schedule,
    memory::calculate_memory_size,
    utils::u256_to_usize,
    vm::VM,
};
use ethrex_common::{
    U256,
    types::Fork,
    utils::{u256_to_big_endian, u256_to_h256},
};

// Stack, Memory, Storage and Flow Operations (15)
// Opcodes: POP, MLOAD, MSTORE, MSTORE8, SLOAD, SSTORE, JUMP, JUMPI, PC, MSIZE, GAS, JUMPDEST, TLOAD, TSTORE, MCOPY

pub const OUT_OF_BOUNDS: U256 = U256([u64::MAX, 0, 0, 0]);

impl<'a> VM<'a> {
    // POP operation
    pub fn op_pop(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::POP)?;
        current_call_frame.stack.pop1()?;
        Ok(OpcodeResult::Continue)
    }

    // TLOAD operation
    pub fn op_tload(&mut self) -> Result<OpcodeResult, VMError> {
        let key = self.current_call_frame.stack.pop1()?;
        let to = self.current_call_frame.to;
        let value = self.substate.get_transient(&to, &key);

        let current_call_frame = &mut self.current_call_frame;

        current_call_frame.increase_consumed_gas(gas_cost::TLOAD)?;

        current_call_frame.stack.push(value)?;
        Ok(OpcodeResult::Continue)
    }

    // TSTORE operation
    pub fn op_tstore(&mut self) -> Result<OpcodeResult, VMError> {
        let (key, value, to) = {
            let current_call_frame = &mut self.current_call_frame;

            current_call_frame.increase_consumed_gas(gas_cost::TSTORE)?;

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
    pub fn op_sload(&mut self) -> Result<OpcodeResult, VMError> {
        let (storage_slot_key, address) = {
            let current_call_frame = &mut self.current_call_frame;
            let storage_slot_key = current_call_frame.stack.pop1()?;
            let address = current_call_frame.to;
            (storage_slot_key, address)
        };

        let storage_slot_key = u256_to_h256(storage_slot_key);

        let (value, storage_slot_was_cold) = self.access_storage_slot(address, storage_slot_key)?;

        let fork = self.env.config.fork;
        let gas = gas_cost::sload_with_fork(storage_slot_was_cold, fork)?;
        self.current_call_frame.increase_consumed_gas(gas)?;

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
        let fork = self.env.config.fork;

        // Fork-aware SSTORE refund values
        // EIP-3529 (London): Reduces SSTORE_CLEARS_SCHEDULE from 15000 to 4800
        // EIP-2929 (Berlin): Uses WARM_STORAGE_READ_COST (100) for restore calculations
        // EIP-2200 (Istanbul): Uses SLOAD_GAS (800) for restore calculations
        let (remove_slot_cost, restore_empty_slot_cost, restore_slot_cost): (u64, u64, u64) =
            if fork >= Fork::London {
                // London (EIP-3529): SSTORE_RESET_GAS + ACCESS_LIST_STORAGE_KEY_COST = 2900 + 1900 = 4800
                // restore_empty = SSTORE_SET_GAS - WARM_STORAGE_READ_COST = 20000 - 100 = 19900
                // restore_slot = SSTORE_RESET_GAS - WARM_STORAGE_READ_COST = 2900 - 100 = 2800
                (4800, 19900, 2800)
            } else if fork >= Fork::Berlin {
                // Berlin (EIP-2929): SSTORE_CLEARS_SCHEDULE unchanged at 15000
                // restore_empty = SSTORE_SET_GAS - WARM_STORAGE_READ_COST = 20000 - 100 = 19900
                // restore_slot = SSTORE_RESET_GAS - WARM_STORAGE_READ_COST = 2900 - 100 = 2800
                (15000, 19900, 2800)
            } else if fork >= Fork::Istanbul {
                // Istanbul (EIP-2200): SSTORE_CLEARS = 15000
                // restore_empty = SSTORE_SET_GAS - SLOAD_GAS = 20000 - 800 = 19200
                // restore_slot = SSTORE_RESET_GAS - SLOAD_GAS = 5000 - 800 = 4200
                (15000, 19200, 4200)
            } else {
                // Pre-Istanbul: SSTORE_CLEARS = 15000
                // Use actual SLOAD gas for the fork (50 for Frontier, 200 for Tangerine+)
                let schedule = gas_schedule::GasSchedule::for_fork(fork);
                let sload_gas = schedule.sload;
                // restore_empty = SSTORE_SET_GAS - SLOAD_GAS
                // restore_slot = SSTORE_RESET_GAS - SLOAD_GAS
                // sload_gas is always <= 200, so these subtractions are safe
                #[expect(clippy::arithmetic_side_effects, reason = "sload_gas is max 200")]
                (15000, 20000 - sload_gas, 5000 - sload_gas)
            };

        // Refund logic depends on fork:
        // - Pre-Constantinople (Frontier through Byzantium, and Petersburg): Simple model
        //   Only clear refund (non-zero → zero) = +15000
        // - Constantinople: EIP-1283 net gas metering (but reverted in Petersburg)
        // - Istanbul+: EIP-2200/EIP-2929/EIP-3529 net gas metering
        let has_net_gas_metering = fork == Fork::Constantinople || fork >= Fork::Istanbul;

        if new_storage_slot_value != current_value {
            if current_value == original_value {
                // First modification from original value
                if !original_value.is_zero() && new_storage_slot_value.is_zero() {
                    // Clearing a slot: add clear refund
                    gas_refunds = gas_refunds
                        .checked_add(remove_slot_cost)
                        .ok_or(InternalError::Overflow)?;
                }
            } else if has_net_gas_metering {
                // Dirty slot handling - only applies with net gas metering (Constantinople, Istanbul+)
                if original_value != U256::zero() {
                    if current_value == U256::zero() {
                        // Was cleared, now being filled - subtract the clear refund
                        gas_refunds = gas_refunds
                            .checked_sub(remove_slot_cost)
                            .ok_or(InternalError::Underflow)?;
                    } else if new_storage_slot_value.is_zero() {
                        // Being cleared now - add clear refund
                        gas_refunds = gas_refunds
                            .checked_add(remove_slot_cost)
                            .ok_or(InternalError::Overflow)?;
                    }
                }
                // Restore to original value - add restore refund
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
            } else {
                // Simple model (pre-Constantinople and Petersburg):
                // For dirty slots (current_value != original_value), still give refund
                // when clearing any non-zero value to zero.
                // This handles the case: orig=0, curr=X, new=0
                if !current_value.is_zero() && new_storage_slot_value.is_zero() {
                    gas_refunds = gas_refunds
                        .checked_add(remove_slot_cost)
                        .ok_or(InternalError::Overflow)?;
                }
            }
        }

        self.substate.refunded_gas = gas_refunds;

        let sstore_cost = gas_cost::sstore_with_fork(
            original_value,
            current_value,
            new_storage_slot_value,
            storage_slot_was_cold,
            fork,
        )?;

        self.current_call_frame.increase_consumed_gas(sstore_cost)?;

        if new_storage_slot_value != current_value {
            self.update_account_storage(to, key, new_storage_slot_value, current_value)?;
        }

        Ok(OpcodeResult::Continue)
    }

    // MSIZE operation
    pub fn op_msize(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::MSIZE)?;
        current_call_frame
            .stack
            .push(current_call_frame.memory.len().into())?;
        Ok(OpcodeResult::Continue)
    }

    // GAS operation
    pub fn op_gas(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::GAS)?;

        let remaining_gas = current_call_frame.gas_remaining;
        // Note: These are not consumed gas calculations, but are related, so I used this wrapping here
        current_call_frame.stack.push(remaining_gas.into())?;

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
        current_call_frame.increase_consumed_gas(gas_cost::JUMP)?;

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
            call_frame.increase_consumed_gas(gas_cost::JUMPDEST)?;
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
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::PC)?;

        current_call_frame
            .stack
            .push(U256::from(current_call_frame.pc.wrapping_sub(1)))?;

        Ok(OpcodeResult::Continue)
    }
}
