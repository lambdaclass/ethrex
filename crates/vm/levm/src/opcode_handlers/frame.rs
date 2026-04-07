//! # EIP-8141 Frame Transaction opcodes
//!
//! Includes:
//!   - `APPROVE` (0xAA)
//!   - `TXPARAMLOAD` (0xB0)
//!   - `TXPARAMSIZE` (0xB1)
//!   - `TXPARAMCOPY` (0xB2)

use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    gas_cost,
    memory::calculate_memory_size,
    opcode_handlers::OpcodeHandler,
    utils::{size_offset_to_usize, u256_to_usize},
    vm::VM,
};
use ethrex_common::{
    Address, H160, U256,
    types::FrameMode,
};

/// The ENTRY_POINT address used as caller for DEFAULT/VERIFY frames.
const ENTRY_POINT_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xaa,
]);

/// Implementation for the `APPROVE` opcode (0xAA).
///
/// Acts like RETURN but updates transaction-scoped approval state.
/// Stack: pop offset, length, scope
pub struct OpApproveHandler;
impl OpcodeHandler for OpApproveHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [offset, len, scope] = *vm.current_call_frame.stack.pop()?;
        let (len, offset) = size_offset_to_usize(len, offset)?;

        // Gas: memory expansion only (APPROVE itself is free)
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::exit_opcode(
                calculate_memory_size(offset, len)?,
                vm.current_call_frame.memory.len(),
            )?)?;

        let ctx = vm
            .env
            .frame_context
            .as_mut()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let current_frame = ctx
            .frames
            .get(ctx.current_frame_index)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        // ADDRESS must equal the current frame's target
        let frame_target = current_frame.target.unwrap_or(ctx.sender);
        if vm.current_call_frame.to != frame_target {
            return Err(ExceptionalHalt::InvalidOpcode.into());
        }

        let scope_val = u256_to_usize(scope)?;
        match scope_val {
            // Scope 0x0: approve execution (sender only)
            0x0 => {
                if ctx.sender_approved {
                    return Err(ExceptionalHalt::InvalidOpcode.into());
                }
                if frame_target != ctx.sender {
                    return Err(ExceptionalHalt::InvalidOpcode.into());
                }
                ctx.sender_approved = true;
            }
            // Scope 0x1: approve payment (needs sender_approved first)
            0x1 => {
                if ctx.payer_approved {
                    return Err(ExceptionalHalt::InvalidOpcode.into());
                }
                if !ctx.sender_approved {
                    return Err(ExceptionalHalt::InvalidOpcode.into());
                }
                // Balance check and nonce increment are handled by the execution loop (T6).
                // Here we just record the approval.
                ctx.payer_approved = true;
                ctx.payer = Some(frame_target);
            }
            // Scope 0x2: approve both (sender only)
            0x2 => {
                if ctx.sender_approved || ctx.payer_approved {
                    return Err(ExceptionalHalt::InvalidOpcode.into());
                }
                if frame_target != ctx.sender {
                    return Err(ExceptionalHalt::InvalidOpcode.into());
                }
                ctx.sender_approved = true;
                ctx.payer_approved = true;
                ctx.payer = Some(frame_target);
            }
            _ => {
                return Err(ExceptionalHalt::InvalidOpcode.into());
            }
        }

        // Behave like RETURN: read return data from memory
        if len != 0 {
            vm.current_call_frame.output = vm.current_call_frame.memory.load_range(offset, len)?;
        }

        Ok(OpcodeResult::Halt)
    }
}

/// Implementation for the `TXPARAMLOAD` opcode (0xB0).
///
/// Reads a transaction parameter and pushes a 32-byte value to the stack.
/// Stack: pop in1 (selector), in2 (index)
pub struct OpTxParamLoadHandler;
impl OpcodeHandler for OpTxParamLoadHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [in1, in2] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TXPARAMLOAD)?;

        let ctx = vm
            .env
            .frame_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let selector = u256_to_usize(in1)?;
        let value = match selector {
            // 0x00: tx_type (always 0x06 for frame transactions)
            0x00 => U256::from(0x06u64),
            // 0x01: nonce
            0x01 => U256::from(vm.env.tx_nonce),
            // 0x02: sender
            0x02 => address_to_u256(ctx.sender),
            // 0x03: max_priority_fee_per_gas
            0x03 => vm.env.tx_max_priority_fee_per_gas.unwrap_or_default(),
            // 0x04: max_fee_per_gas
            0x04 => vm.env.tx_max_fee_per_gas.unwrap_or_default(),
            // 0x05: max_fee_per_blob_gas
            0x05 => vm.env.tx_max_fee_per_blob_gas.unwrap_or_default(),
            // 0x06: max_cost (sum of all frame gas_limits * max_fee_per_gas)
            0x06 => {
                let total_gas: u64 = ctx.frames.iter().map(|f| f.gas_limit).sum();
                U256::from(total_gas)
                    .checked_mul(vm.env.tx_max_fee_per_gas.unwrap_or_default())
                    .unwrap_or(U256::max_value())
            }
            // 0x07: len(blob_versioned_hashes)
            0x07 => U256::from(vm.env.tx_blob_hashes.len()),
            // 0x08: sig_hash
            0x08 => {
                let bytes = ctx.sig_hash.0;
                U256::from_big_endian(&bytes)
            }
            // 0x09: len(frames)
            0x09 => U256::from(ctx.frames.len()),
            // 0x10: current_frame_index
            0x10 => U256::from(ctx.current_frame_index),
            // 0x11: frame_target(in2)
            0x11 => {
                let frame_idx = u256_to_usize(in2)?;
                let frame = ctx
                    .frames
                    .get(frame_idx)
                    .ok_or(ExceptionalHalt::InvalidOpcode)?;
                match frame.target {
                    Some(addr) => address_to_u256(addr),
                    None => address_to_u256(ctx.sender),
                }
            }
            // 0x12: frame_data(in2) -- returns first 32 bytes; VERIFY frames return empty
            0x12 => {
                let frame_idx = u256_to_usize(in2)?;
                let frame = ctx
                    .frames
                    .get(frame_idx)
                    .ok_or(ExceptionalHalt::InvalidOpcode)?;
                if frame.mode == FrameMode::Verify {
                    U256::zero()
                } else {
                    let data = &frame.data;
                    if data.is_empty() {
                        U256::zero()
                    } else {
                        let mut bytes = [0u8; 32];
                        let copy_len = data.len().min(32);
                        bytes[..copy_len].copy_from_slice(&data[..copy_len]);
                        U256::from_big_endian(&bytes)
                    }
                }
            }
            // 0x13: frame_gas_limit(in2)
            0x13 => {
                let frame_idx = u256_to_usize(in2)?;
                let frame = ctx
                    .frames
                    .get(frame_idx)
                    .ok_or(ExceptionalHalt::InvalidOpcode)?;
                U256::from(frame.gas_limit)
            }
            // 0x14: frame_mode(in2)
            0x14 => {
                let frame_idx = u256_to_usize(in2)?;
                let frame = ctx
                    .frames
                    .get(frame_idx)
                    .ok_or(ExceptionalHalt::InvalidOpcode)?;
                U256::from(frame.mode as u8)
            }
            // 0x15: frame_status(in2) -- 0=not-yet-executed, 1=success, 2=failure
            0x15 => {
                let frame_idx = u256_to_usize(in2)?;
                if frame_idx >= ctx.frames.len() {
                    return Err(ExceptionalHalt::InvalidOpcode.into());
                }
                match ctx.frame_results.get(frame_idx) {
                    Some(Some(result)) => {
                        if result.success {
                            U256::from(1u64)
                        } else {
                            U256::from(2u64)
                        }
                    }
                    _ => U256::zero(),
                }
            }
            _ => {
                return Err(ExceptionalHalt::InvalidOpcode.into());
            }
        };

        vm.current_call_frame.stack.push(value)?;
        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `TXPARAMSIZE` opcode (0xB1).
///
/// Returns the size of a transaction parameter.
/// Stack: pop in1 (selector), in2 (index)
pub struct OpTxParamSizeHandler;
impl OpcodeHandler for OpTxParamSizeHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [in1, in2] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TXPARAMSIZE)?;

        let ctx = vm
            .env
            .frame_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let selector = u256_to_usize(in1)?;
        let size = match selector {
            // Most parameters are 32 bytes
            0x00..=0x11 | 0x13..=0x15 => 32usize,
            // 0x12: frame_data size is dynamic
            0x12 => {
                let frame_idx = u256_to_usize(in2)?;
                let frame = ctx
                    .frames
                    .get(frame_idx)
                    .ok_or(ExceptionalHalt::InvalidOpcode)?;
                if frame.mode == FrameMode::Verify {
                    0
                } else {
                    frame.data.len()
                }
            }
            _ => {
                return Err(ExceptionalHalt::InvalidOpcode.into());
            }
        };

        vm.current_call_frame.stack.push(U256::from(size))?;
        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `TXPARAMCOPY` opcode (0xB2).
///
/// Copies transaction parameter data to memory.
/// Stack: pop in1, in2, dest_offset, offset, size
pub struct OpTxParamCopyHandler;
impl OpcodeHandler for OpTxParamCopyHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [in1, in2, dest_offset, src_offset, size] = *vm.current_call_frame.stack.pop()?;
        let (size, dest_offset) = size_offset_to_usize(size, dest_offset)?;
        let src_offset = u256_to_usize(src_offset).unwrap_or(usize::MAX);

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::calldatacopy(
                calculate_memory_size(dest_offset, size)?,
                vm.current_call_frame.memory.len(),
                size,
            )?)?;

        if size == 0 {
            return Ok(OpcodeResult::Continue);
        }

        let ctx = vm
            .env
            .frame_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        // Get the parameter data based on selector
        let selector = u256_to_usize(in1)?;
        let param_data = get_param_data(ctx, &vm.env, selector, in2)?;

        let data = param_data.get(src_offset..).unwrap_or_default();
        let data = if data.len() >= size {
            &data[..size]
        } else {
            data
        };

        vm.current_call_frame.memory.store_data(dest_offset, data)?;
        if data.len() < size {
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "data.len() < size guard ensures no underflow"
            )]
            vm.current_call_frame
                .memory
                .store_zeros(dest_offset + data.len(), size - data.len())?;
        }

        Ok(OpcodeResult::Continue)
    }
}

/// Convert an Address to U256 (left-padded with zeros).
fn address_to_u256(addr: Address) -> U256 {
    let mut bytes = [0u8; 32];
    bytes[12..].copy_from_slice(&addr.0);
    U256::from_big_endian(&bytes)
}

/// Get parameter data as a byte vector for TXPARAMCOPY.
fn get_param_data(
    ctx: &crate::environment::FrameExecutionContext,
    env: &crate::environment::Environment,
    selector: usize,
    in2: U256,
) -> Result<Vec<u8>, VMError> {
    match selector {
        0x00 => Ok(u256_to_bytes32(U256::from(0x06u64))),
        0x01 => Ok(u256_to_bytes32(U256::from(env.tx_nonce))),
        0x02 => Ok(u256_to_bytes32(address_to_u256(ctx.sender))),
        0x03 => Ok(u256_to_bytes32(
            env.tx_max_priority_fee_per_gas.unwrap_or_default(),
        )),
        0x04 => Ok(u256_to_bytes32(
            env.tx_max_fee_per_gas.unwrap_or_default(),
        )),
        0x05 => Ok(u256_to_bytes32(
            env.tx_max_fee_per_blob_gas.unwrap_or_default(),
        )),
        0x06 => {
            let total_gas: u64 = ctx.frames.iter().map(|f| f.gas_limit).sum();
            Ok(u256_to_bytes32(
                U256::from(total_gas)
                    .checked_mul(env.tx_max_fee_per_gas.unwrap_or_default())
                    .unwrap_or(U256::max_value()),
            ))
        }
        0x07 => Ok(u256_to_bytes32(U256::from(env.tx_blob_hashes.len()))),
        0x08 => Ok(ctx.sig_hash.0.to_vec()),
        0x09 => Ok(u256_to_bytes32(U256::from(ctx.frames.len()))),
        0x10 => Ok(u256_to_bytes32(U256::from(ctx.current_frame_index))),
        0x11 => {
            let frame_idx = u256_to_usize(in2)?;
            let frame = ctx
                .frames
                .get(frame_idx)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            let addr = frame.target.unwrap_or(ctx.sender);
            Ok(u256_to_bytes32(address_to_u256(addr)))
        }
        0x12 => {
            let frame_idx = u256_to_usize(in2)?;
            let frame = ctx
                .frames
                .get(frame_idx)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            if frame.mode == FrameMode::Verify {
                Ok(Vec::new())
            } else {
                Ok(frame.data.to_vec())
            }
        }
        0x13 => {
            let frame_idx = u256_to_usize(in2)?;
            let frame = ctx
                .frames
                .get(frame_idx)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            Ok(u256_to_bytes32(U256::from(frame.gas_limit)))
        }
        0x14 => {
            let frame_idx = u256_to_usize(in2)?;
            let frame = ctx
                .frames
                .get(frame_idx)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            Ok(u256_to_bytes32(U256::from(frame.mode as u8)))
        }
        0x15 => {
            let frame_idx = u256_to_usize(in2)?;
            if frame_idx >= ctx.frames.len() {
                return Err(ExceptionalHalt::InvalidOpcode.into());
            }
            let status = match ctx.frame_results.get(frame_idx) {
                Some(Some(result)) => {
                    if result.success {
                        1u64
                    } else {
                        2u64
                    }
                }
                _ => 0u64,
            };
            Ok(u256_to_bytes32(U256::from(status)))
        }
        _ => Err(ExceptionalHalt::InvalidOpcode.into()),
    }
}

/// Convert a U256 to 32-byte big-endian representation.
fn u256_to_bytes32(value: U256) -> Vec<u8> {
    value.to_big_endian().to_vec()
}
