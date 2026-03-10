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
use ethrex_common::{H256, U256};
use std::{mem, slice};

/// Implementation for the `POP` opcode.
pub struct OpPopHandler;
impl OpcodeHandler for OpPopHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::POP)?;

        vm.current_call_frame.stack.pop1()?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `GAS` opcode.
pub struct OpGasHandler;
impl OpcodeHandler for OpGasHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::GAS)?;

        vm.current_call_frame
            .stack
            .push(vm.current_call_frame.gas_remaining.into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `PC` opcode.
pub struct OpPcHandler;
impl OpcodeHandler for OpPcHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::PC)?;

        // Note: Since the PC has been preincremented, subtracting 1 from it to get the operation's
        //   offset will never cause an underflow condition.
        vm.current_call_frame
            .stack
            .push(vm.current_call_frame.pc.wrapping_sub(1).into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `MLOAD` opcode.
pub struct OpMLoadHandler;
impl OpcodeHandler for OpMLoadHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let offset = u256_to_usize(vm.current_call_frame.stack.pop1()?)?;
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::mload(
                calculate_memory_size(offset, WORD_SIZE_IN_BYTES_USIZE)?,
                vm.current_call_frame.memory.len(),
            )?)?;

        vm.current_call_frame
            .stack
            .push(vm.current_call_frame.memory.load_word(offset)?)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `MSTORE` opcode.
pub struct OpMStoreHandler;
impl OpcodeHandler for OpMStoreHandler {
    #[inline(always)]
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
    #[inline(always)]
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
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [dst_offset, src_offset, len] = *vm.current_call_frame.stack.pop()?;
        let (len, dst_offset) = size_offset_to_usize(len, dst_offset)?;
        let src_offset = u256_to_usize(src_offset).unwrap_or(usize::MAX);

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::mcopy(
                calculate_memory_size(src_offset.max(dst_offset), len)?,
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
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::MSIZE)?;

        vm.current_call_frame
            .stack
            .push(vm.current_call_frame.memory.len().into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `TLOAD` opcode.
pub struct OpTLoadHandler;
impl OpcodeHandler for OpTLoadHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TLOAD)?;

        let key = vm.current_call_frame.stack.pop1()?;
        vm.current_call_frame
            .stack
            .push(vm.substate.get_transient(&vm.current_call_frame.to, &key))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `TSTORE` opcode.
pub struct OpTStoreHandler;
impl OpcodeHandler for OpTStoreHandler {
    #[inline(always)]
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
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let storage_slot_key = vm.current_call_frame.stack.pop1()?;
        let address = vm.current_call_frame.to;
        let key = {
            #[expect(unsafe_code)]
            unsafe {
                let mut hash = mem::transmute::<U256, H256>(storage_slot_key);
                hash.0.reverse();
                hash
            }
        };

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::sload(
                vm.substate.add_accessed_slot(address, key),
            )?)?;

        // Record to BAL AFTER gas check passes per EIP-7928
        vm.record_storage_slot_to_bal(address, storage_slot_key);

        let value = vm.get_storage_value(address, key)?;
        vm.current_call_frame.stack.push(value)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SSTORE` opcode.
pub struct OpSStoreHandler;
impl OpcodeHandler for OpSStoreHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        if vm.current_call_frame.is_static {
            return Err(ExceptionalHalt::OpcodeNotAllowedInStaticContext.into());
        }

        // EIP-2200
        if vm.current_call_frame.gas_remaining <= SSTORE_STIPEND {
            return Err(ExceptionalHalt::OutOfGas.into());
        }

        let [storage_slot_key, value] = *vm.current_call_frame.stack.pop()?;
        let to = vm.current_call_frame.to;
        #[expect(unsafe_code)]
        let key = unsafe {
            let mut hash = mem::transmute::<U256, H256>(storage_slot_key);
            hash.0.reverse();
            hash
        };

        let current_value = vm.get_storage_value(to, key)?;
        let original_value = vm.get_original_storage(to, key)?;

        // Record storage read to BAL AFTER SSTORE_STIPEND check passes, BEFORE main gas check.
        // Per EIP-7928: if SSTORE passes the stipend check but fails the main gas charge,
        // the slot MUST appear as a read because the implicit SLOAD has already happened.
        vm.record_storage_slot_to_bal(to, storage_slot_key);

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::sstore(
                original_value,
                current_value,
                value,
                vm.substate.add_accessed_slot(to, key),
            )?)?;
        if value != current_value {
            // EIP-2929
            const REMOVE_SLOT_COST: i64 = 4800;
            const RESTORE_EMPTY_SLOT_COST: i64 = 19900;
            const RESTORE_SLOT_COST: i64 = 2800;

            // The operations on `delta` cannot overflow.
            let mut delta = 0i64;
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "delta additions are bounded by known constants"
            )]
            if current_value == original_value {
                if !original_value.is_zero() && value.is_zero() {
                    delta += REMOVE_SLOT_COST;
                }
            } else {
                if !original_value.is_zero() {
                    if current_value.is_zero() {
                        delta -= REMOVE_SLOT_COST;
                    } else if value.is_zero() {
                        delta += REMOVE_SLOT_COST;
                    }
                }

                if value == original_value {
                    if original_value.is_zero() {
                        delta += RESTORE_EMPTY_SLOT_COST;
                    } else {
                        delta += RESTORE_SLOT_COST;
                    }
                }
            }

            // Update refunded gas after checking for overflow or underflow.
            match vm.substate.refunded_gas.checked_add_signed(delta) {
                Some(refunded_gas) => vm.substate.refunded_gas = refunded_gas,
                None if delta < 0 => return Err(InternalError::Underflow.into()),
                None => return Err(InternalError::Overflow.into()),
            }
        }

        if value != current_value {
            vm.update_account_storage(to, key, storage_slot_key, value, current_value)?;
        }

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `JUMPDEST` opcode.
pub struct OpJumpDestHandler;
impl OpcodeHandler for OpJumpDestHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::JUMPDEST)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `JUMP` opcode.
pub struct OpJumpHandler;
impl OpcodeHandler for OpJumpHandler {
    #[inline(always)]
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
    #[inline(always)]
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
    #[expect(clippy::as_conversions, reason = "safe")]
    if vm
        .current_call_frame
        .bytecode
        .bytecode
        .get(target)
        .is_some_and(|&value| {
            value == Opcode::JUMPDEST as u8
                && vm
                    .current_call_frame
                    .bytecode
                    .jump_targets
                    .binary_search(&(target as u32))
                    .is_ok()
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
