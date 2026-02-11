//! # Environment operations
//!
//! Includes the following opcodes:
//!   - `ADDRESS`
//!   - `BALANCE`
//!   - `ORIGIN`
//!   - `GASPRICE`
//!   - `CALLER`
//!   - `CALLVALUE`
//!   - `CALLDATALOAD`
//!   - `CALLDATASIZE`
//!   - `CALLDATACOPY`
//!   - `CODESIZE`
//!   - `CODECOPY`
//!   - `EXTCODESIZE`
//!   - `EXTCODECOPY`
//!   - `EXTCODEHASH`
//!   - `RETURNDATASIZE`
//!   - `RETURNDATACOPY`

use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    gas_cost::{self},
    memory::calculate_memory_size,
    opcode_handlers::OpcodeHandler,
    utils::{size_offset_to_usize, u256_to_usize, word_to_address},
    vm::VM,
};
use ethrex_common::U256;
use std::mem;

/// Implementation for the `ADDRESS` opcode.
pub struct OpAddressHandler;
impl OpcodeHandler for OpAddressHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::ADDRESS)?;

        #[expect(unsafe_code, reason = "safe")]
        vm.current_call_frame.stack.push(U256(unsafe {
            let mut bytes: [u8; 32] = [0; 32];
            bytes[12..].copy_from_slice(&vm.current_call_frame.to.0);
            bytes.reverse();
            mem::transmute_copy::<[u8; 32], [u64; 4]>(&bytes)
        }))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `BALANCE` opcode.
pub struct OpBalanceHandler;
impl OpcodeHandler for OpBalanceHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(vm.current_call_frame.stack.pop1()?);
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::balance(
                vm.substate.add_accessed_address(address),
            )?)?;

        // State access AFTER gas check passes
        let account_balance = vm.db.get_account(address)?.info.balance;

        // Record address touch for BAL (after gas check passes)
        if let Some(recorder) = vm.db.bal_recorder.as_mut() {
            recorder.record_touched_address(address);
        }

        vm.current_call_frame.stack.push(account_balance)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `ORIGIN` opcode.
pub struct OpOriginHandler;
impl OpcodeHandler for OpOriginHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::ORIGIN)?;

        #[expect(unsafe_code, reason = "safe")]
        vm.current_call_frame.stack.push(U256(unsafe {
            let mut bytes: [u8; 32] = [0; 32];
            bytes[12..].copy_from_slice(&vm.env.origin.0);
            bytes.reverse();
            mem::transmute_copy::<[u8; 32], [u64; 4]>(&bytes)
        }))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `GASPRICE` opcode.
pub struct OpGasPriceHandler;
impl OpcodeHandler for OpGasPriceHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::GASPRICE)?;

        vm.current_call_frame.stack.push(vm.env.gas_price)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `CALLER` opcode.
pub struct OpCallerHandler;
impl OpcodeHandler for OpCallerHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::CALLER)?;

        #[expect(unsafe_code, reason = "safe")]
        vm.current_call_frame.stack.push(U256(unsafe {
            let mut bytes: [u8; 32] = [0; 32];
            bytes[12..].copy_from_slice(&vm.current_call_frame.msg_sender.0);
            bytes.reverse();
            mem::transmute_copy::<[u8; 32], [u64; 4]>(&bytes)
        }))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `CALLVALUE` opcode.
pub struct OpCallValueHandler;
impl OpcodeHandler for OpCallValueHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::CALLVALUE)?;

        vm.current_call_frame
            .stack
            .push(vm.current_call_frame.msg_value)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `CALLDATALOAD` opcode.
pub struct OpCallDataLoadHandler;
impl OpcodeHandler for OpCallDataLoadHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::CALLDATALOAD)?;

        let value_bytes = usize::try_from(vm.current_call_frame.stack.pop1()?)
            .ok()
            .and_then(|offset| vm.current_call_frame.calldata.get(offset..));
        vm.current_call_frame.stack.push(match value_bytes {
            Some(data) if data.len() >= 32 => U256::from_big_endian(&data[..32]),
            Some(data) => {
                let mut bytes = [0; 32];
                bytes[..data.len()].copy_from_slice(data);
                U256::from_big_endian(&bytes)
            }
            None => U256::zero(),
        })?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `CALLDATASIZE` opcode.
pub struct OpCallDataSizeHandler;
impl OpcodeHandler for OpCallDataSizeHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::CALLDATASIZE)?;

        vm.current_call_frame
            .stack
            .push(U256::from(vm.current_call_frame.calldata.len()))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `CALLDATACOPY` opcode.
pub struct OpCallDataCopyHandler;
impl OpcodeHandler for OpCallDataCopyHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [dst_offset, src_offset, len] = *vm.current_call_frame.stack.pop()?;
        let (len, dst_offset) = size_offset_to_usize(len, dst_offset)?;
        let src_offset = u256_to_usize(src_offset).unwrap_or(usize::MAX);

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::calldatacopy(
                calculate_memory_size(dst_offset, len)?,
                vm.current_call_frame.memory.len(),
                len,
            )?)?;

        if len > 0 {
            let data = vm
                .current_call_frame
                .calldata
                .get(src_offset..)
                .unwrap_or_default();
            let data = data.get(..len).unwrap_or(data);

            vm.current_call_frame.memory.store_data(dst_offset, data)?;
            if data.len() < len {
                vm.current_call_frame
                    .memory
                    .store_zeros(dst_offset + data.len(), len - data.len())?;
            }
        }

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `CODESIZE` opcode.
pub struct OpCodeSizeHandler;
impl OpcodeHandler for OpCodeSizeHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::CODESIZE)?;

        vm.current_call_frame
            .stack
            .push(vm.current_call_frame.bytecode.bytecode.len().into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `CODECOPY` opcode.
pub struct OpCodeCopyHandler;
impl OpcodeHandler for OpCodeCopyHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [dst_offset, src_offset, len] = *vm.current_call_frame.stack.pop()?;
        let (len, dst_offset) = size_offset_to_usize(len, dst_offset)?;
        let src_offset = u256_to_usize(src_offset).unwrap_or(usize::MAX);

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::codecopy(
                calculate_memory_size(dst_offset, len)?,
                vm.current_call_frame.memory.len(),
                len,
            )?)?;

        if len > 0 {
            let data = vm
                .current_call_frame
                .bytecode
                .bytecode
                .get(src_offset..)
                .unwrap_or_default();
            let data = data.get(..len).unwrap_or(data);

            vm.current_call_frame.memory.store_data(dst_offset, data)?;
            if data.len() < len {
                vm.current_call_frame
                    .memory
                    .store_zeros(dst_offset + data.len(), len - data.len())?;
            }
        }

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `EXTCODESIZE` opcode.
pub struct OpExtCodeSizeHandler;
impl OpcodeHandler for OpExtCodeSizeHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(vm.current_call_frame.stack.pop1()?);
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::extcodesize(
                vm.substate.add_accessed_address(address),
            )?)?;

        // State access AFTER gas check passes (using optimized code length lookup)
        let account_code_length = vm.db.get_code_length(address)?.into();

        // Record address touch for BAL (after gas check passes)
        if let Some(recorder) = vm.db.bal_recorder.as_mut() {
            recorder.record_touched_address(address);
        }

        vm.current_call_frame.stack.push(account_code_length)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `EXTCODECOPY` opcode.
pub struct OpExtCodeCopyHandler;
impl OpcodeHandler for OpExtCodeCopyHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [address, dst_offset, src_offset, len] = *vm.current_call_frame.stack.pop()?;
        let address = word_to_address(address);
        let (len, dst_offset) = size_offset_to_usize(len, dst_offset)?;
        let src_offset = u256_to_usize(src_offset).unwrap_or(usize::MAX);

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::extcodecopy(
                len,
                calculate_memory_size(dst_offset, len)?,
                vm.current_call_frame.memory.len(),
                vm.substate.add_accessed_address(address),
            )?)?;

        // Record address touch for BAL (after gas check passes)
        if let Some(recorder) = vm.db.bal_recorder.as_mut() {
            recorder.record_touched_address(address);
        }

        if len > 0 {
            let data = vm
                .db
                .get_account_code(address)?
                .bytecode
                .get(src_offset..)
                .unwrap_or_default();
            let data = data.get(..len).unwrap_or(data);

            vm.current_call_frame.memory.store_data(dst_offset, data)?;
            if data.len() < len {
                vm.current_call_frame
                    .memory
                    .store_zeros(dst_offset + data.len(), len - data.len())?;
            }
        }

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `EXTCODEHASH` opcode.
pub struct OpExtCodeHashHandler;
impl OpcodeHandler for OpExtCodeHashHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let address = word_to_address(vm.current_call_frame.stack.pop1()?);
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::extcodehash(
                vm.substate.add_accessed_address(address),
            )?)?;

        let account = vm.db.get_account(address)?;
        let account_is_empty = account.is_empty();
        let account_code_hash = account.info.code_hash.0;

        // Record address touch for BAL (after gas check passes)
        if let Some(recorder) = vm.db.bal_recorder.as_mut() {
            recorder.record_touched_address(address);
        }

        if account_is_empty {
            vm.current_call_frame.stack.push_zero()?;
        } else {
            #[expect(unsafe_code, reason = "safe")]
            vm.current_call_frame.stack.push(U256(unsafe {
                let mut bytes = account_code_hash;
                bytes.reverse();
                mem::transmute_copy::<[u8; 32], [u64; 4]>(&bytes)
            }))?;
        }

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `RETURNDATASIZE` opcode.
pub struct OpReturnDataSizeHandler;
impl OpcodeHandler for OpReturnDataSizeHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::RETURNDATASIZE)?;

        vm.current_call_frame
            .stack
            .push(vm.current_call_frame.sub_return_data.len().into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `RETURNDATACOPY` opcode.
pub struct OpReturnDataCopyHandler;
impl OpcodeHandler for OpReturnDataCopyHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [dst_offset, src_offset, len] = *vm.current_call_frame.stack.pop()?;
        let (len, dst_offset) = size_offset_to_usize(len, dst_offset)?;
        let src_offset = u256_to_usize(src_offset)?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::returndatacopy(
                calculate_memory_size(dst_offset, len)?,
                vm.current_call_frame.memory.len(),
                len,
            )?)?;

        if src_offset + len > vm.current_call_frame.sub_return_data.len() {
            return Err(ExceptionalHalt::OutOfBounds.into());
        }

        if len > 0 {
            let data = vm
                .current_call_frame
                .sub_return_data
                .get(src_offset..)
                .unwrap_or_default();
            let data = data.get(..len).unwrap_or(data);

            vm.current_call_frame.memory.store_data(dst_offset, data)?;
            if data.len() < len {
                vm.current_call_frame
                    .memory
                    .store_zeros(dst_offset + data.len(), len - data.len())?;
            }
        }

        Ok(OpcodeResult::Continue)
    }
}
