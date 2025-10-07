//! # Control flow and memory operations
//!
//! Includes the following opcodes:
//!   - `POP`
//!   - `GAS`
//!   - `PC`
//!   - `MLOAD`
//!   - `MSTORE`
//!   - `MSTORE8`
//!   - `MCOPY`
//!   - `MSIZE`
//!   - `TLOAD`
//!   - `TSTORE`
//!   - `SLOAD`
//!   - `SSTORE`
//!   - `JUMPDEST`
//!   - `JUMP`
//!   - `JUMPI`

use std::{mem, slice};

use crate::{
    constants::WORD_SIZE_IN_BYTES_USIZE,
    errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError},
    gas_cost::{self, SSTORE_STIPEND},
    memory::calculate_memory_size,
    opcode_handlers::OpcodeHandler,
    opcodes::Opcode,
    utils::{size_offset_to_usize, u256_to_usize},
    vm::VM,
};
use ethrex_common::{H256, U256, utils::u256_to_h256};

/// Implementation for the `POP` opcode.
pub struct OpPopHandler;
impl OpcodeHandler for OpPopHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::POP)?;

        vm.current_call_frame.stack.pop1()?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `GAS` opcode.
pub struct OpGasHandler;
impl OpcodeHandler for OpGasHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::GAS)?;

        vm.current_call_frame
            .stack
            .push1(vm.current_call_frame.gas_remaining.into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `PC` opcode.
pub struct OpPcHandler;
impl OpcodeHandler for OpPcHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::PC)?;

        // Note: Since the PC has been preincremented, subtracting 1 from it to get the operation's
        //   offset will never cause an underflow condition.
        vm.current_call_frame
            .stack
            .push1(vm.current_call_frame.pc.wrapping_sub(1).into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `MLOAD` opcode.
pub struct OpMLoadHandler;
impl OpcodeHandler for OpMLoadHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let offset = u256_to_usize(vm.current_call_frame.stack.pop1()?)?;
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::mload(
                calculate_memory_size(offset, WORD_SIZE_IN_BYTES_USIZE)?,
                vm.current_call_frame.memory.len(),
            )?)?;

        vm.current_call_frame
            .stack
            .push1(vm.current_call_frame.memory.load_word(offset)?)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `MSTORE` opcode.
pub struct OpMStoreHandler;
impl OpcodeHandler for OpMStoreHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [offset, value] = *vm.current_call_frame.stack.pop()?;

        // Handle debug text printing for solidity contracts that enable it.
        if vm.debug_mode.enabled && vm.debug_mode.handle_debug(offset, value)? {
            return Ok(OpcodeResult::Continue);
        }

        let offset = u256_to_usize(offset)?;
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::mstore(
                calculate_memory_size(offset, WORD_SIZE_IN_BYTES_USIZE)?,
                vm.current_call_frame.memory.len(),
            )?)?;

        vm.current_call_frame.memory.store_word(offset, value)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `MSTORE8` opcode.
pub struct OpMStore8Handler;
impl OpcodeHandler for OpMStore8Handler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [offset, value] = *vm.current_call_frame.stack.pop()?;
        let offset = u256_to_usize(offset)?;
        let value = value.byte(0);

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::mstore8(
                calculate_memory_size(offset, size_of::<u8>())?,
                vm.current_call_frame.memory.len(),
            )?)?;

        vm.current_call_frame
            .memory
            .store_data(offset, slice::from_ref(&value))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `MCOPY` opcode.
pub struct OpMCopyHandler;
impl OpcodeHandler for OpMCopyHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [dst_offset, src_offset, len] = *vm.current_call_frame.stack.pop()?;
        let (len, dst_offset) = size_offset_to_usize(len, dst_offset)?;
        let src_offset = u256_to_usize(src_offset).unwrap_or(usize::MAX);

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::mcopy(
                calculate_memory_size(dst_offset, len)?,
                vm.current_call_frame.memory.len(),
                len,
            )?)?;

        vm.current_call_frame
            .memory
            .copy_within(src_offset, dst_offset, len)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `MSIZE` opcode.
pub struct OpMSizeHandler;
impl OpcodeHandler for OpMSizeHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::MSIZE)?;

        vm.current_call_frame
            .stack
            .push1(vm.current_call_frame.memory.len().into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `TLOAD` opcode.
pub struct OpTLoadHandler;
impl OpcodeHandler for OpTLoadHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TLOAD)?;

        let key = vm.current_call_frame.stack.pop1()?;
        vm.current_call_frame
            .stack
            .push1(vm.substate.get_transient(&vm.current_call_frame.to, &key))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `TSTORE` opcode.
pub struct OpTStoreHandler;
impl OpcodeHandler for OpTStoreHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        if vm.current_call_frame.is_static {
            return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
        }

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TSTORE)?;

        let [key, value] = *vm.current_call_frame.stack.pop()?;
        vm.substate
            .set_transient(&vm.current_call_frame.to, &key, value);

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SLOAD` opcode.
pub struct OpSLoadHandler;
impl OpcodeHandler for OpSLoadHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let key = {
            let key = vm.current_call_frame.stack.pop1()?;
            unsafe { mem::transmute::<U256, H256>(key) }
        };
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::sload(
                vm.substate.add_accessed_slot(vm.current_call_frame.to, key),
            )?)?;

        let value = vm.get_storage_value(vm.current_call_frame.to, key)?;
        vm.current_call_frame.stack.push1(value)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SSTORE` opcode.
pub struct OpSStoreHandler;
impl OpcodeHandler for OpSStoreHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        if vm.current_call_frame.is_static {
            return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
        }

        // EIP-2200
        if vm.current_call_frame.gas_remaining <= SSTORE_STIPEND {
            return Err(ExceptionalHalt::OutOfGas.into());
        }

        // vm.current_call_frame
        //     .increase_consumed_gas(gas_cost::sstore(
        //         original_value,
        //         current_value,
        //         new_value,
        //         storage_slot_was_cold,
        //     )?);

        todo!()
    }
}

/// Implementation for the `JUMPDEST` opcode.
pub struct OpJumpDestHandler;
impl OpcodeHandler for OpJumpDestHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::JUMPDEST)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `JUMP` opcode.
pub struct OpJumpHandler;
impl OpcodeHandler for OpJumpHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::JUMP)?;

        let target = vm.current_call_frame.stack.pop1()?;
        jump(vm, target.try_into().unwrap_or(usize::MAX))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `JUMPI` opcode.
pub struct OpJumpIHandler;
impl OpcodeHandler for OpJumpIHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::JUMPI)?;

        let [target, condition] = *vm.current_call_frame.stack.pop()?;
        if !condition.is_zero() {
            jump(vm, target.try_into().unwrap_or(usize::MAX))?;
        }

        Ok(OpcodeResult::Continue)
    }
}

fn jump(vm: &mut VM<'_>, target: usize) -> Result<(), VMError> {
    // Check target address validity.
    //   - Target bytecode has to be a JUMPDEST.
    //   - Target address must not be blacklisted (aka. the JUMPDEST must not be part of a literal).
    if vm
        .current_call_frame
        .bytecode
        .get(target)
        .is_some_and(|&value| {
            value == Opcode::JUMPDEST as u8
                && !vm
                    .current_call_frame
                    .jump_target_filter
                    .is_blacklisted(target)
        })
    {
        // Update PC and skip the JUMPDEST instruction.
        vm.current_call_frame.pc = target.wrapping_add(1);
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::JUMPDEST)?;

        Ok(())
    } else {
        // Target address is invalid.
        Err(ExceptionalHalt::InvalidJump.into())
    }
}

impl<'a> VM<'a> {
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
}
