//! # EIP-8141 Frame transaction opcodes
//!
//! Includes the following opcodes:
//!   - `APPROVE`
//!   - `TXPARAMLOAD`
//!   - `TXPARAMSIZE`
//!   - `TXPARAMCOPY`

use crate::{
    errors::{OpcodeResult, VMError},
    gas_cost,
    opcode_handlers::OpcodeHandler,
    vm::VM,
};
use ethrex_common::U256;

/// Implementation for the `APPROVE` opcode (0xAA).
///
/// Pops scope, length, offset from the stack.
/// scope=0: approve sender, scope=1: approve payer, scope=2: approve both.
/// Halts execution after approval.
pub struct OpApproveHandler;
impl OpcodeHandler for OpApproveHandler {
    // Required by OpcodeHandler trait - not JavaScript eval()
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let scope = vm.current_call_frame.stack.pop1()?;
        let _length = vm.current_call_frame.stack.pop1()?;
        let _offset = vm.current_call_frame.stack.pop1()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::APPROVE)?;

        let ctx = vm
            .env
            .frame_context
            .as_mut()
            .ok_or(VMError::Internal(
                crate::errors::InternalError::Custom("APPROVE: not in a frame transaction".into()),
            ))?;

        let scope_val: u64 = scope
            .try_into()
            .map_err(|_| VMError::Internal(
                crate::errors::InternalError::Custom("APPROVE: invalid scope".into()),
            ))?;

        match scope_val {
            0 => ctx.sender_approved = true,
            1 => ctx.payer_approved = true,
            2 => {
                ctx.sender_approved = true;
                ctx.payer_approved = true;
            }
            _ => {
                return Err(VMError::Internal(
                    crate::errors::InternalError::Custom("APPROVE: invalid scope value".into()),
                ));
            }
        }

        Ok(OpcodeResult::Halt)
    }
}

/// Implementation for the `TXPARAMLOAD` opcode (0xB0).
///
/// Pops in1 (param id) and in2 (sub-index) from the stack.
/// Pushes the requested transaction parameter value.
pub struct OpTxParamLoadHandler;
impl OpcodeHandler for OpTxParamLoadHandler {
    // Required by OpcodeHandler trait - not JavaScript eval()
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let in1 = vm.current_call_frame.stack.pop1()?;
        let _in2 = vm.current_call_frame.stack.pop1()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TXPARAMLOAD)?;

        let ctx = vm
            .env
            .frame_context
            .as_ref()
            .ok_or(VMError::Internal(
                crate::errors::InternalError::Custom("TXPARAMLOAD: not in a frame transaction".into()),
            ))?;

        let param_id: u64 = in1.try_into().unwrap_or(u64::MAX);

        let result = match param_id {
            // 0x00: tx type (0x06 for frame tx)
            0x00 => U256::from(0x06),
            // 0x01: nonce
            0x01 => U256::from(vm.env.tx_nonce),
            // 0x02: sender address
            0x02 => {
                let mut bytes = [0u8; 32];
                bytes[12..].copy_from_slice(&ctx.sender.0);
                U256::from_big_endian(&bytes)
            }
            // 0x08: sig_hash
            0x08 => {
                U256::from_big_endian(&ctx.sig_hash.0)
            }
            // 0x09: number of frames
            0x09 => U256::from(ctx.frames.len()),
            // 0x10: current frame index
            0x10 => U256::from(ctx.current_frame_index),
            // Default: return 0 for unrecognized params in PoC
            _ => U256::zero(),
        };

        vm.current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `TXPARAMSIZE` opcode (0xB1).
///
/// Pops in1 (param id) and in2 (sub-index) from the stack.
/// Pushes the size of the requested parameter (32 for most scalar params).
pub struct OpTxParamSizeHandler;
impl OpcodeHandler for OpTxParamSizeHandler {
    // Required by OpcodeHandler trait - not JavaScript eval()
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let in1 = vm.current_call_frame.stack.pop1()?;
        let _in2 = vm.current_call_frame.stack.pop1()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TXPARAMSIZE)?;

        let _ctx = vm
            .env
            .frame_context
            .as_ref()
            .ok_or(VMError::Internal(
                crate::errors::InternalError::Custom("TXPARAMSIZE: not in a frame transaction".into()),
            ))?;

        let param_id: u64 = in1.try_into().unwrap_or(u64::MAX);

        // Most scalar parameters are 32 bytes
        let size = match param_id {
            0x00..=0x10 => U256::from(32),
            _ => U256::zero(),
        };

        vm.current_call_frame.stack.push(size)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `TXPARAMCOPY` opcode (0xB2).
///
/// Simplified PoC: just charges gas and returns Continue.
pub struct OpTxParamCopyHandler;
impl OpcodeHandler for OpTxParamCopyHandler {
    // Required by OpcodeHandler trait - not JavaScript eval()
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        // Pop dest_offset, src_offset, length from stack
        let _dest_offset = vm.current_call_frame.stack.pop1()?;
        let _src_offset = vm.current_call_frame.stack.pop1()?;
        let _length = vm.current_call_frame.stack.pop1()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TXPARAMCOPY_STATIC)?;

        Ok(OpcodeResult::Continue)
    }
}
