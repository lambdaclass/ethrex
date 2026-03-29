//! # EIP-8141 Frame Transaction opcodes
//!
//! Includes:
//!   - `APPROVE` (0xAA)
//!   - `TXPARAM` (0xB0)
//!   - `FRAMEDATALOAD` (0xB1)
//!   - `FRAMEDATACOPY` (0xB2)
//!   - Default code for EOAs

use crate::{
    call_frame::CallFrame,
    errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError},
    gas_cost,
    memory::{Memory, calculate_memory_size},
    opcode_handlers::OpcodeHandler,
    precompiles,
    utils::size_offset_to_usize,
    vm::VM,
};
use bytes::Bytes;
use ethrex_common::{Address, U256, types::FrameMode, types::Log};
use ethrex_rlp::{decode::RLPDecode, error::RLPDecodeError, structs::Decoder};
use std::mem;

/// Convert a u64 index to usize, returning InvalidOpcode on overflow.
fn index_to_usize(val: u64) -> Result<usize, VMError> {
    usize::try_from(val).map_err(|_| ExceptionalHalt::InvalidOpcode.into())
}

/// Compute the max transaction cost for APPROVE payment logic.
fn compute_max_tx_cost(ctx: &crate::vm::FrameTxContext) -> Result<U256, VMError> {
    let halt_err: VMError = ExceptionalHalt::InvalidOpcode.into();
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
    max_tx_cost
        .checked_add(blob_fee)
        .ok_or(ExceptionalHalt::InvalidOpcode.into())
}

/// Apply APPROVE side effects for the given scope.
/// This is shared between OpApproveHandler and (future) default code.
pub fn apply_approve(vm: &mut VM<'_>, scope: u64, frame_target: ethrex_common::Address) -> Result<(), VMError> {
    let halt_err: VMError = ExceptionalHalt::InvalidOpcode.into();

    match scope {
        0x1 => {
            // Sender approval only
            let ctx = vm.frame_tx_context.as_ref().ok_or(halt_err.clone())?;
            if ctx.sender_approved {
                return Err(halt_err);
            }
            if frame_target != ctx.tx.sender {
                return Err(VMError::RevertOpcode);
            }
            let ctx = vm.frame_tx_context.as_mut().ok_or(ExceptionalHalt::InvalidOpcode)?;
            ctx.sender_approved = true;
        }
        0x2 => {
            // Payer approval only
            let ctx = vm.frame_tx_context.as_ref().ok_or(halt_err.clone())?;
            if ctx.payer_approved {
                return Err(halt_err.clone());
            }
            if !ctx.sender_approved {
                return Err(VMError::RevertOpcode);
            }
            let max_tx_cost = compute_max_tx_cost(ctx)?;
            let sender = ctx.tx.sender;

            vm.increment_account_nonce(sender)?;
            vm.decrease_account_balance(frame_target, max_tx_cost)?;

            let ctx = vm.frame_tx_context.as_mut().ok_or(ExceptionalHalt::InvalidOpcode)?;
            ctx.payer_approved = true;
            ctx.payer_address = Some(frame_target);
        }
        0x3 => {
            // Combined sender + payer approval
            let ctx = vm.frame_tx_context.as_ref().ok_or(halt_err.clone())?;
            if ctx.sender_approved || ctx.payer_approved {
                return Err(halt_err.clone());
            }
            if frame_target != ctx.tx.sender {
                return Err(VMError::RevertOpcode);
            }
            let max_tx_cost = compute_max_tx_cost(ctx)?;
            let sender = ctx.tx.sender;

            vm.increment_account_nonce(sender)?;
            vm.decrease_account_balance(frame_target, max_tx_cost)?;

            let ctx = vm.frame_tx_context.as_mut().ok_or(ExceptionalHalt::InvalidOpcode)?;
            ctx.sender_approved = true;
            ctx.payer_approved = true;
            ctx.payer_address = Some(frame_target);
        }
        _ => {
            // scope 0 and any other value are invalid
            return Err(halt_err);
        }
    }
    Ok(())
}

/// APPROVE (0xAA) -- Frame transaction approval opcode.
///
/// Pops [offset, length, scope] from the stack.
/// - scope 0x1: sender approval (validate sender identity)
/// - scope 0x2: payer approval (deduct gas cost from payer)
/// - scope 0x3: combined sender + payer approval
/// - scope 0x0 and others: invalid (exceptional halt)
///
/// Scope restriction from mode bits 8-9: if nonzero, only that scope is allowed.
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
        let current_frame = ctx
            .frames
            .get(ctx.current_frame_index)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;
        let frame_target = current_frame.target.unwrap_or(ctx.tx.sender);
        if vm.current_call_frame.to != frame_target {
            return Err(VMError::RevertOpcode);
        }

        // Enforce scope restriction from mode bits 8-9
        let scope_restriction = current_frame.scope_restriction();
        let scope_val = scope.as_u64();
        if scope_restriction != 0 && scope_val != scope_restriction as u64 {
            return Err(ExceptionalHalt::InvalidOpcode.into());
        }

        // Charge gas (memory expansion, same as RETURN)
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::exit_opcode(
                calculate_memory_size(offset, length)?,
                vm.current_call_frame.memory.len(),
            )?)?;

        apply_approve(vm, scope_val, frame_target)?;

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

/// TXPARAM (0xB0) -- Load a transaction parameter as a 32-byte word.
/// Gas cost: 2
pub struct OpTxParamHandler;
impl OpcodeHandler for OpTxParamHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [param_id, index] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TXPARAM)?;

        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let result = load_tx_param(ctx, param_id.as_u64(), index.as_u64())?;
        vm.current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }
}

/// FRAMEDATALOAD (0xB1) -- Load one 32-byte word from a frame's data.
/// Takes [offset, frameIndex] from the stack. Gas cost: 3.
/// VERIFY frames return zero.
pub struct OpFrameDataLoadHandler;
impl OpcodeHandler for OpFrameDataLoadHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [offset, frame_index] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::FRAMEDATALOAD)?;

        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let idx = index_to_usize(frame_index.as_u64())?;
        let frame = ctx
            .frames
            .get(idx)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        // VERIFY frames return zero
        if frame.execution_mode() == FrameMode::Verify {
            vm.current_call_frame.stack.push(U256::zero())?;
            return Ok(OpcodeResult::Continue);
        }

        let byte_offset = offset.as_u64() as usize;
        let data = &frame.data;
        let mut word = [0u8; 32];
        let available = data.len().saturating_sub(byte_offset);
        let copy_len = available.min(32);
        if copy_len > 0 {
            if let Some(src) = data.get(byte_offset..byte_offset + copy_len) {
                word[..copy_len].copy_from_slice(src);
            }
        }

        vm.current_call_frame
            .stack
            .push(U256::from_big_endian(&word))?;

        Ok(OpcodeResult::Continue)
    }
}

/// FRAMEDATACOPY (0xB2) -- Copy frame data into memory.
/// Takes [memOffset, dataOffset, length, frameIndex] from the stack.
/// Gas cost matches CALLDATACOPY. VERIFY frames copy nothing.
pub struct OpFrameDataCopyHandler;
impl OpcodeHandler for OpFrameDataCopyHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [mem_offset, data_offset, length, frame_index] =
            *vm.current_call_frame.stack.pop()?;
        let (length, mem_offset) = size_offset_to_usize(length, mem_offset)?;
        let data_offset = index_to_usize(data_offset.as_u64())?;

        let new_memory_size = calculate_memory_size(mem_offset, length)?;
        let current_memory_size = vm.current_call_frame.memory.len();
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::framedatacopy(
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

        let idx = index_to_usize(frame_index.as_u64())?;
        let frame = ctx
            .frames
            .get(idx)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        // VERIFY frames copy nothing (zero-fill)
        if frame.execution_mode() == FrameMode::Verify {
            let buf = vec![0u8; length];
            vm.current_call_frame.memory.store_data(mem_offset, &buf)?;
            return Ok(OpcodeResult::Continue);
        }

        let data = &frame.data;
        let mut buf = vec![0u8; length];
        let available = data.len().saturating_sub(data_offset);
        let copy_len = length.min(available);
        if let (Some(dst), Some(src)) = (
            buf.get_mut(..copy_len),
            data.get(data_offset..data_offset.saturating_add(copy_len)),
        ) {
            dst.copy_from_slice(src);
        }

        vm.current_call_frame.memory.store_data(mem_offset, &buf)?;

        Ok(OpcodeResult::Continue)
    }
}

// -- Helper functions --

fn load_tx_param(
    ctx: &crate::vm::FrameTxContext,
    param_id: u64,
    index: u64,
) -> Result<U256, VMError> {
    match param_id {
        // Scalar parameters (index must be 0)
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
            // gas_limit
            let frame = ctx
                .frames
                .get(index_to_usize(index)?)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            Ok(U256::from(frame.gas_limit))
        }
        0x13 => {
            // mode (lower 8 bits only)
            let frame = ctx
                .frames
                .get(index_to_usize(index)?)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            Ok(U256::from(frame.mode & 0xFF))
        }
        0x14 => {
            // len(data) -- returns 0 for VERIFY frames
            let frame = ctx
                .frames
                .get(index_to_usize(index)?)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            if frame.execution_mode() == FrameMode::Verify {
                Ok(U256::zero())
            } else {
                Ok(U256::from(frame.data.len()))
            }
        }
        0x15 => {
            // status -- exceptional halt if current/future frame
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
        0x16 => {
            // scope (bits 8-9 from mode)
            let frame = ctx
                .frames
                .get(index_to_usize(index)?)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            Ok(U256::from(frame.scope_restriction()))
        }
        0x17 => {
            // atomic_batch (bit 10 from mode, returns 0 or 1)
            let frame = ctx
                .frames
                .get(index_to_usize(index)?)
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            Ok(U256::from(frame.is_atomic_batch() as u8))
        }
        _ => Err(ExceptionalHalt::InvalidOpcode.into()),
    }
}

fn address_to_u256(addr: ethrex_common::Address) -> U256 {
    let mut bytes = [0u8; 32];
    bytes[12..].copy_from_slice(addr.as_bytes());
    U256::from_big_endian(&bytes)
}

// -- Default code for EOAs (EIP-8141) --

/// A single subcall in SENDER mode default code.
/// RLP-encoded as [target, value, data].
struct SenderCall {
    target: Address,
    value: U256,
    data: Bytes,
}

impl RLPDecode for SenderCall {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (target, decoder) = decoder.decode_field("target")?;
        let (value, decoder) = decoder.decode_field("value")?;
        let (data, decoder) = decoder.decode_field("data")?;
        let rest = decoder.finish()?;
        Ok((SenderCall { target, value, data }, rest))
    }
}

/// Execute default code for an EOA target in a frame transaction.
///
/// When a frame targets an address with no deployed code (an EOA), the protocol
/// runs built-in "default code" instead of executing a normal CALL.
///
/// Returns `(success, gas_used, logs)`.
pub fn execute_default_code(
    vm: &mut VM<'_>,
    frame: &ethrex_common::types::Frame,
    sender: Address,
    target: Address,
) -> Result<(bool, u64, Vec<Log>), VMError> {
    match frame.execution_mode() {
        FrameMode::Verify => execute_default_verify(vm, frame, target),
        FrameMode::Sender => execute_default_sender(vm, frame, sender, target),
        FrameMode::Default => Ok((false, 0, Vec::new())),
    }
}

/// VERIFY mode default code: validate a signature and call APPROVE.
fn execute_default_verify(
    vm: &mut VM<'_>,
    frame: &ethrex_common::types::Frame,
    target: Address,
) -> Result<(bool, u64, Vec<Log>), VMError> {
    let ctx = vm
        .frame_tx_context
        .as_ref()
        .ok_or(ExceptionalHalt::InvalidOpcode)?;

    // frame.target must be tx.sender
    if target != ctx.tx.sender {
        return Ok((false, 0, Vec::new()));
    }

    // Read scope from mode bits 8-9
    let scope = frame.scope_restriction() as u64;
    if scope == 0 {
        return Ok((false, 0, Vec::new()));
    }

    // Need at least 1 byte for signature_type
    if frame.data.is_empty() {
        return Ok((false, 0, Vec::new()));
    }
    let signature_type = frame.data[0];
    let sig_hash = ctx.sig_hash;

    let mut gas_remaining = frame.gas_limit;

    match signature_type {
        // secp256k1
        0x0 => {
            // data layout: [type(1), v(1), r(32), s(32)] = 66 bytes
            if frame.data.len() != 66 {
                return Ok((false, 0, Vec::new()));
            }
            let v = frame.data[1];
            let r = &frame.data[2..34];
            let s = &frame.data[34..66];

            // Build ecrecover calldata: [hash(32), v_padded(32), r(32), s(32)]
            let mut calldata = vec![0u8; 128];
            calldata[..32].copy_from_slice(sig_hash.as_bytes());
            // v goes in the last byte of the second 32-byte word
            calldata[63] = v;
            calldata[64..96].copy_from_slice(r);
            calldata[96..128].copy_from_slice(s);

            let result = precompiles::ecrecover(
                &Bytes::from(calldata),
                &mut gas_remaining,
                vm.env.config.fork,
            )?;

            // Check recovered address matches target (result is 32-byte padded address)
            if result.len() != 32 || result[12..] != *target.as_bytes() {
                return Ok((false, frame.gas_limit - gas_remaining, Vec::new()));
            }
        }
        // P256
        0x1 => {
            // data layout: [type(1), r(32), s(32), qx(32), qy(32)] = 129 bytes
            if frame.data.len() != 129 {
                return Ok((false, 0, Vec::new()));
            }
            let r = &frame.data[1..33];
            let s = &frame.data[33..65];
            let qx = &frame.data[65..97];
            let qy = &frame.data[97..129];

            // Check frame.target == keccak256(qx || qy)[12:]
            let mut pubkey_bytes = [0u8; 64];
            pubkey_bytes[..32].copy_from_slice(qx);
            pubkey_bytes[32..].copy_from_slice(qy);
            let hash = ethrex_crypto::keccak::keccak_hash(&pubkey_bytes);
            let derived_address = Address::from_slice(&hash[12..]);
            if target != derived_address {
                return Ok((false, 0, Vec::new()));
            }

            // Build P256VERIFY calldata: [hash(32), r(32), s(32), qx(32), qy(32)]
            let mut calldata = vec![0u8; 160];
            calldata[..32].copy_from_slice(sig_hash.as_bytes());
            calldata[32..64].copy_from_slice(r);
            calldata[64..96].copy_from_slice(s);
            calldata[96..128].copy_from_slice(qx);
            calldata[128..160].copy_from_slice(qy);

            let result = precompiles::p_256_verify(
                &Bytes::from(calldata),
                &mut gas_remaining,
                vm.env.config.fork,
            )?;

            // P256VERIFY returns 32-byte value with 1 on success, empty on failure
            if result.len() != 32 || result[31] != 1 {
                return Ok((false, frame.gas_limit - gas_remaining, Vec::new()));
            }
        }
        // Unknown signature type
        _ => return Ok((false, 0, Vec::new())),
    }

    let gas_used = frame.gas_limit - gas_remaining;

    // Call APPROVE
    apply_approve(vm, scope, target)?;

    // Mark approve as called in the current frame
    let ctx = vm
        .frame_tx_context
        .as_mut()
        .ok_or(ExceptionalHalt::InvalidOpcode)?;
    ctx.approve_called_in_current_frame = true;

    Ok((true, gas_used, Vec::new()))
}

/// SENDER mode default code: execute subcalls as tx.sender.
fn execute_default_sender(
    vm: &mut VM<'_>,
    frame: &ethrex_common::types::Frame,
    sender: Address,
    target: Address,
) -> Result<(bool, u64, Vec<Log>), VMError> {
    let ctx = vm
        .frame_tx_context
        .as_ref()
        .ok_or(ExceptionalHalt::InvalidOpcode)?;

    // frame.target must be tx.sender
    if target != ctx.tx.sender {
        return Ok((false, 0, Vec::new()));
    }

    // Decode frame.data as RLP [[target, value, data], ...]
    let calls = Vec::<SenderCall>::decode(&frame.data).map_err(|_| {
        VMError::Internal(InternalError::Custom(
            "invalid RLP in SENDER default code data".to_string(),
        ))
    })?;

    let mut gas_remaining = frame.gas_limit;
    let mut all_logs: Vec<Log> = Vec::new();

    for call in &calls {
        // Charge address access cost (warm/cold)
        let is_cold = vm.substate.add_accessed_address(call.target);
        let access_cost = if is_cold {
            gas_cost::COLD_ADDRESS_ACCESS_COST
        } else {
            gas_cost::WARM_ADDRESS_ACCESS_COST
        };

        if gas_remaining < access_cost {
            return Ok((false, frame.gas_limit, Vec::new()));
        }
        gas_remaining -= access_cost;

        // Get target's bytecode
        let bytecode = vm.db.get_account_code(call.target)?.clone();

        // Allocate gas for subcall using 63/64 rule (EIP-150)
        let subcall_gas = gas_remaining - gas_remaining / 64;

        let call_frame = CallFrame::new(
            sender,            // msg_sender = tx.sender per spec
            call.target,       // to
            call.target,       // code_address
            bytecode,          // bytecode
            call.value,        // msg_value
            call.data.clone(), // calldata
            false,             // is_static
            subcall_gas,       // gas_limit
            0,                 // depth
            !call.value.is_zero(), // should_transfer_value
            false,             // is_create
            0,                 // ret_offset
            0,                 // ret_size
            vm.stack_pool.pop().unwrap_or_default(),
            Memory::default(),
        );

        // Save and swap in the subcall frame
        let saved_call_frame = mem::replace(&mut vm.current_call_frame, call_frame);
        let saved_call_frames = mem::take(&mut vm.call_frames);

        vm.substate.push_backup();
        let subcall_result = vm.run_execution();

        let (success, subcall_gas_used) = match subcall_result {
            Ok(ctx_result) => {
                let gas_used = ctx_result.gas_used;
                if ctx_result.is_success() {
                    vm.substate.commit_backup();
                    let logs = vm.substate.extract_logs();
                    for log in &logs {
                        vm.substate.add_log(log.clone());
                    }
                    all_logs.extend(logs);
                    (true, gas_used)
                } else {
                    vm.substate.revert_backup();
                    vm.restore_cache_state()?;
                    (false, gas_used)
                }
            }
            Err(_) => {
                vm.substate.revert_backup();
                vm.restore_cache_state()?;
                (false, subcall_gas)
            }
        };

        // Restore call frame state
        let finished_frame = mem::replace(&mut vm.current_call_frame, saved_call_frame);
        vm.call_frames = saved_call_frames;
        vm.stack_pool.push(finished_frame.stack);

        gas_remaining = gas_remaining.saturating_sub(subcall_gas_used);

        if !success {
            return Ok((false, frame.gas_limit - gas_remaining, Vec::new()));
        }
    }

    Ok((true, frame.gas_limit - gas_remaining, all_logs))
}
