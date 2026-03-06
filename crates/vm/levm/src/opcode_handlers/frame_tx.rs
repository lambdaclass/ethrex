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
    utils::size_offset_to_usize,
    vm::VM,
};
use ethrex_common::{U256, types::FrameMode};

/// Convert a u64 index to usize, returning InvalidOpcode on overflow.
fn index_to_usize(val: u64) -> Result<usize, VMError> {
    usize::try_from(val).map_err(|_| ExceptionalHalt::InvalidOpcode.into())
}

/// APPROVE (0xAA) — Frame transaction approval opcode.
///
/// Pops [offset, length, scope] from the stack.
/// - scope 0x0: sender approval (validate sender identity)
/// - scope 0x1: payer approval (deduct gas cost from payer)
/// - scope 0x2: combined sender + payer approval
///
/// On success, copies memory[offset..offset+length] to output and halts the frame.
pub struct OpApproveHandler;
impl OpcodeHandler for OpApproveHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [offset, length, scope] = *vm.current_call_frame.stack.pop()?;
        let (length, offset) = size_offset_to_usize(length, offset)?;

        // Must be in a frame transaction context
        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        // The executing contract must be the frame's target
        let frame_target = ctx
            .frames
            .get(ctx.current_frame_index)
            .ok_or(ExceptionalHalt::InvalidOpcode)?
            .target
            .unwrap_or(ctx.tx.sender);
        if vm.current_call_frame.to != frame_target {
            return Err(VMError::RevertOpcode);
        }

        // Charge gas (memory expansion, same as RETURN)
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::exit_opcode(
                calculate_memory_size(offset, length)?,
                vm.current_call_frame.memory.len(),
            )?)?;

        let scope_val = scope.as_u64();

        let halt_err: VMError = ExceptionalHalt::InvalidOpcode.into();

        match scope_val {
            0x0 => {
                // Sender approval only
                let ctx = vm.frame_tx_context.as_ref().ok_or(halt_err.clone())?;
                if ctx.sender_approved {
                    return Err(halt_err);
                }
                if frame_target != ctx.tx.sender {
                    return Err(VMError::RevertOpcode);
                }
                let ctx = vm.frame_tx_context.as_mut().ok_or(halt_err)?;
                ctx.sender_approved = true;
            }
            0x1 => {
                // Payer approval only
                let ctx = vm.frame_tx_context.as_ref().ok_or(halt_err.clone())?;
                if ctx.payer_approved {
                    return Err(halt_err);
                }
                if !ctx.sender_approved {
                    return Err(VMError::RevertOpcode);
                }
                let max_fee = U256::from(ctx.tx.max_fee_per_gas);
                let gas_limit = U256::from(ctx.tx.total_gas_limit());
                let max_tx_cost = max_fee.checked_mul(gas_limit).ok_or(halt_err.clone())?;
                let blob_count = U256::from(ctx.tx.blob_versioned_hashes.len());
                let gas_per_blob = U256::from(131072u64); // GAS_PER_BLOB from EIP-4844
                let blob_fee = blob_count
                    .checked_mul(gas_per_blob)
                    .ok_or(halt_err.clone())?
                    .checked_mul(ctx.tx.max_fee_per_blob_gas)
                    .ok_or(halt_err.clone())?;
                let max_tx_cost = max_tx_cost.checked_add(blob_fee).ok_or(halt_err.clone())?;
                let sender = ctx.tx.sender;

                vm.increment_account_nonce(sender)?;
                vm.decrease_account_balance(frame_target, max_tx_cost)?;

                let ctx = vm.frame_tx_context.as_mut().ok_or(halt_err)?;
                ctx.payer_approved = true;
                ctx.payer_address = Some(frame_target);
            }
            0x2 => {
                // Combined sender + payer approval
                let ctx = vm.frame_tx_context.as_ref().ok_or(halt_err.clone())?;
                if ctx.sender_approved || ctx.payer_approved {
                    return Err(halt_err);
                }
                if frame_target != ctx.tx.sender {
                    return Err(VMError::RevertOpcode);
                }
                let max_fee = U256::from(ctx.tx.max_fee_per_gas);
                let gas_limit = U256::from(ctx.tx.total_gas_limit());
                let max_tx_cost = max_fee.checked_mul(gas_limit).ok_or(halt_err.clone())?;
                let blob_count = U256::from(ctx.tx.blob_versioned_hashes.len());
                let gas_per_blob = U256::from(131072u64); // GAS_PER_BLOB from EIP-4844
                let blob_fee = blob_count
                    .checked_mul(gas_per_blob)
                    .ok_or(halt_err.clone())?
                    .checked_mul(ctx.tx.max_fee_per_blob_gas)
                    .ok_or(halt_err.clone())?;
                let max_tx_cost = max_tx_cost.checked_add(blob_fee).ok_or(halt_err.clone())?;
                let sender = ctx.tx.sender;

                vm.increment_account_nonce(sender)?;
                vm.decrease_account_balance(frame_target, max_tx_cost)?;

                let ctx = vm.frame_tx_context.as_mut().ok_or(halt_err)?;
                ctx.sender_approved = true;
                ctx.payer_approved = true;
                ctx.payer_address = Some(frame_target);
            }
            _ => {
                return Err(halt_err);
            }
        }

        let ctx = vm
            .frame_tx_context
            .as_mut()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;
        ctx.approve_called_in_current_frame = true;

        // Copy memory to output (like RETURN)
        if length != 0 {
            vm.current_call_frame.output =
                vm.current_call_frame.memory.load_range(offset, length)?;
        }

        Ok(OpcodeResult::Halt)
    }
}

/// TXPARAMLOAD (0xB0) — Load a transaction parameter as a 32-byte word.
pub struct OpTxParamLoadHandler;
impl OpcodeHandler for OpTxParamLoadHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [param_id, index] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TXPARAMLOAD)?;

        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let result = load_tx_param(ctx, param_id.as_u64(), index.as_u64())?;
        vm.current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }
}

/// TXPARAMSIZE (0xB1) — Get the size of a transaction parameter.
pub struct OpTxParamSizeHandler;
impl OpcodeHandler for OpTxParamSizeHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [param_id, index] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TXPARAMSIZE)?;

        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let size = get_tx_param_size(ctx, param_id.as_u64(), index.as_u64())?;
        vm.current_call_frame.stack.push(U256::from(size))?;

        Ok(OpcodeResult::Continue)
    }
}

/// TXPARAMCOPY (0xB2) — Copy transaction parameter data to memory.
pub struct OpTxParamCopyHandler;
impl OpcodeHandler for OpTxParamCopyHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [param_id, index, dest_offset, src_offset, length] =
            *vm.current_call_frame.stack.pop()?;
        let (length, dest_offset) = size_offset_to_usize(length, dest_offset)?;
        let src_offset = index_to_usize(src_offset.as_u64())?;

        let new_memory_size = calculate_memory_size(dest_offset, length)?;
        let current_memory_size = vm.current_call_frame.memory.len();
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::txparamcopy(
                new_memory_size,
                current_memory_size,
                length,
            )?)?;

        if length == 0 {
            return Ok(OpcodeResult::Continue);
        }

        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let data = get_tx_param_data(ctx, param_id.as_u64(), index.as_u64())?;

        // Copy from data[src_offset..], zero-padding if out of bounds
        let mut buf = vec![0u8; length];
        let available = data.len().saturating_sub(src_offset);
        let copy_len = length.min(available);
        if let (Some(dst), Some(src)) = (
            buf.get_mut(..copy_len),
            data.get(src_offset..src_offset.saturating_add(copy_len)),
        ) {
            dst.copy_from_slice(src);
        }

        vm.current_call_frame.memory.store_data(dest_offset, &buf)?;

        Ok(OpcodeResult::Continue)
    }
}

// ── Helper functions ──

fn load_tx_param(
    ctx: &crate::vm::FrameTxContext,
    param_id: u64,
    index: u64,
) -> Result<U256, VMError> {
    match param_id {
        // Scalar parameters
        0x00 => Ok(U256::from(0x06u8)), // tx_type (EIP-8141 = type 6)
        0x01 => Ok(U256::from(ctx.tx.nonce)),
        0x02 => Ok(address_to_u256(ctx.tx.sender)),
        0x03 => Ok(U256::from(ctx.tx.max_priority_fee_per_gas)),
        0x04 => Ok(U256::from(ctx.tx.max_fee_per_gas)),
        0x05 => Ok(ctx.tx.max_fee_per_blob_gas),
        0x06 => {
            // max_cost = max_fee_per_gas * total_gas_limit + len(blob_hashes) * 131072 * max_fee_per_blob_gas
            let gas_cost = U256::from(ctx.tx.max_fee_per_gas)
                .checked_mul(U256::from(ctx.tx.total_gas_limit()))
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            let blob_cost = U256::from(ctx.tx.blob_versioned_hashes.len())
                .checked_mul(U256::from(131072u64))
                .ok_or(ExceptionalHalt::InvalidOpcode)?
                .checked_mul(ctx.tx.max_fee_per_blob_gas)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            gas_cost
                .checked_add(blob_cost)
                .ok_or(ExceptionalHalt::InvalidOpcode.into())
        }
        0x07 => Ok(U256::from(ctx.tx.blob_versioned_hashes.len())),
        0x08 => {
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(ctx.sig_hash.as_bytes());
            Ok(U256::from_big_endian(&bytes))
        }
        0x09 => Ok(U256::from(ctx.frames.len())),

        // Per-frame parameters (index = frame index)
        0x10 => Ok(U256::from(ctx.current_frame_index)),
        0x11 => {
            let frame = ctx
                .frames
                .get(index_to_usize(index)?)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            Ok(address_to_u256(frame.target.unwrap_or(ctx.tx.sender)))
        }
        0x12 => {
            let frame = ctx
                .frames
                .get(index_to_usize(index)?)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            if frame.mode == FrameMode::Verify {
                return Ok(U256::zero());
            }
            let data = &frame.data;
            if data.is_empty() {
                return Ok(U256::zero());
            }
            let mut bytes = [0u8; 32];
            let copy_len = data.len().min(32);
            if let (Some(dst), Some(src)) = (bytes.get_mut(..copy_len), data.get(..copy_len)) {
                dst.copy_from_slice(src);
            }
            Ok(U256::from_big_endian(&bytes))
        }
        0x13 => {
            let frame = ctx
                .frames
                .get(index_to_usize(index)?)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            Ok(U256::from(frame.gas_limit))
        }
        0x14 => {
            let frame = ctx
                .frames
                .get(index_to_usize(index)?)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            Ok(U256::from(u8::from(frame.mode)))
        }
        0x15 => {
            let idx = index_to_usize(index)?;
            if idx >= ctx.current_frame_index {
                return Err(ExceptionalHalt::InvalidOpcode.into());
            }
            let (success, _, _) = ctx
                .frame_results
                .get(idx)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            Ok(if *success { U256::one() } else { U256::zero() })
        }
        _ => Err(ExceptionalHalt::InvalidOpcode.into()),
    }
}

fn get_tx_param_size(
    ctx: &crate::vm::FrameTxContext,
    param_id: u64,
    index: u64,
) -> Result<usize, VMError> {
    match param_id {
        0x00..=0x09 => Ok(32),
        0x10 | 0x11 | 0x13 | 0x14 | 0x15 => Ok(32),
        0x12 => {
            let frame = ctx
                .frames
                .get(index_to_usize(index)?)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            if frame.mode == FrameMode::Verify {
                Ok(0)
            } else {
                Ok(frame.data.len())
            }
        }
        _ => Err(ExceptionalHalt::InvalidOpcode.into()),
    }
}

fn get_tx_param_data(
    ctx: &crate::vm::FrameTxContext,
    param_id: u64,
    index: u64,
) -> Result<Vec<u8>, VMError> {
    match param_id {
        0x00..=0x09 | 0x10 | 0x11 | 0x13 | 0x14 | 0x15 => {
            let val = load_tx_param(ctx, param_id, index)?;
            Ok(val.to_big_endian().to_vec())
        }
        0x12 => {
            let frame = ctx
                .frames
                .get(index_to_usize(index)?)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            if frame.mode == FrameMode::Verify {
                Ok(vec![])
            } else {
                Ok(frame.data.to_vec())
            }
        }
        _ => Err(ExceptionalHalt::InvalidOpcode.into()),
    }
}

fn address_to_u256(addr: ethrex_common::Address) -> U256 {
    let mut bytes = [0u8; 32];
    bytes[12..].copy_from_slice(addr.as_bytes());
    U256::from_big_endian(&bytes)
}
