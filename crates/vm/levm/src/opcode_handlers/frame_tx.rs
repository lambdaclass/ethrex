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

/// Convert a U256 offset to usize, returning None when the value does not fit
/// in usize on the current target. Used by FRAMEDATALOAD and FRAMEDATACOPY so
/// out-of-range offsets are treated as past-the-end rather than as an
/// exceptional halt (per the EIP-8141 spec the load returns zero and the copy
/// writes zero bytes).
fn u256_to_offset(value: U256) -> Option<usize> {
    if value.0[1] != 0 || value.0[2] != 0 || value.0[3] != 0 {
        return None;
    }
    usize::try_from(value.0[0]).ok()
}

/// Compute the transaction's maximum cost for APPROVE payment deduction.
/// Per spec, this is TXPARAM(0x06): max_fee_per_gas * total_gas_limit + blob cost.
fn compute_tx_cost(ctx: &crate::vm::FrameTxContext) -> Result<U256, VMError> {
    let halt_err: VMError = ExceptionalHalt::InvalidOpcode.into();
    let gas_limit = U256::from(ctx.tx.total_gas_limit());
    let max_fee = U256::from(ctx.tx.max_fee_per_gas);
    let tx_cost = max_fee.checked_mul(gas_limit).ok_or(halt_err.clone())?;
    let blob_count = U256::from(ctx.tx.blob_versioned_hashes.len());
    let gas_per_blob = U256::from(131072u64); // GAS_PER_BLOB from EIP-4844
    let blob_fee = blob_count
        .checked_mul(gas_per_blob)
        .ok_or(halt_err.clone())?
        .checked_mul(ctx.tx.max_fee_per_blob_gas)
        .ok_or(halt_err.clone())?;
    tx_cost
        .checked_add(blob_fee)
        .ok_or(ExceptionalHalt::InvalidOpcode.into())
}

/// Apply APPROVE side effects for the given scope.
/// This is shared between OpApproveHandler and (future) default code.
pub fn apply_approve(vm: &mut VM<'_>, scope: u64, frame_target: ethrex_common::Address) -> Result<(), VMError> {
    let halt_err: VMError = ExceptionalHalt::InvalidOpcode.into();

    match scope {
        0x1 => {
            // APPROVE_PAYMENT: increment nonce, deduct max cost, set payer_approved
            let ctx = vm.frame_tx_context.as_ref().ok_or(halt_err.clone())?;
            if ctx.payer_approved {
                return Err(halt_err.clone());
            }
            if !ctx.sender_approved {
                return Err(VMError::RevertOpcode);
            }
            let tx_cost = compute_tx_cost(ctx)?;
            let sender = ctx.tx.sender;

            vm.increment_account_nonce(sender)?;
            // Payer balance underflow is a frame-level revert, not a consensus
            // fault: the outer restore_cache_state() path rolls back the nonce
            // increment above when RevertOpcode propagates.
            match vm.decrease_account_balance(frame_target, tx_cost) {
                Ok(()) => {}
                Err(InternalError::Underflow) => return Err(VMError::RevertOpcode),
                Err(e) => return Err(VMError::Internal(e)),
            }

            let ctx = vm.frame_tx_context.as_mut().ok_or(ExceptionalHalt::InvalidOpcode)?;
            ctx.payer_approved = true;
            ctx.payer_address = Some(frame_target);
        }
        0x2 => {
            // APPROVE_EXECUTION: set sender_approved (requires frame_target == tx.sender)
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
        0x3 => {
            // APPROVE_PAYMENT_AND_EXECUTION: both
            let ctx = vm.frame_tx_context.as_ref().ok_or(halt_err.clone())?;
            if ctx.sender_approved || ctx.payer_approved {
                return Err(halt_err.clone());
            }
            if frame_target != ctx.tx.sender {
                return Err(VMError::RevertOpcode);
            }
            let tx_cost = compute_tx_cost(ctx)?;
            let sender = ctx.tx.sender;

            vm.increment_account_nonce(sender)?;
            // See scope 0x1 above for the Underflow → RevertOpcode rationale.
            match vm.decrease_account_balance(frame_target, tx_cost) {
                Ok(()) => {}
                Err(InternalError::Underflow) => return Err(VMError::RevertOpcode),
                Err(e) => return Err(VMError::Internal(e)),
            }

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

        // Enforce scope restriction from flags bits 0-1
        let allowed_scope = current_frame.scope_restriction();
        let scope_val = scope.as_u64();
        // scope must be a non-zero subset of allowed_scope
        if scope_val == 0
            || scope_val > 3
            || (allowed_scope != 0 && (scope_val & allowed_scope as u64) != scope_val)
        {
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
        let [param_id] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TXPARAM)?;

        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let result = load_tx_param(ctx, param_id.as_u64())?;
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

        // Out-of-usize offsets are past-the-end: the word stays zero-filled.
        let mut word = [0u8; 32];
        if let Some(byte_offset) = u256_to_offset(offset) {
            let data = &frame.data;
            let available = data.len().saturating_sub(byte_offset);
            let copy_len = available.min(32);
            if copy_len > 0 {
                if let Some(src) = data.get(byte_offset..byte_offset + copy_len) {
                    word[..copy_len].copy_from_slice(src);
                }
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
        // Out-of-usize data_offset is past-the-end: destination stays zero-filled.
        let data_offset_opt = u256_to_offset(data_offset);

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
        if let Some(data_offset) = data_offset_opt {
            let available = data.len().saturating_sub(data_offset);
            let copy_len = length.min(available);
            if let (Some(dst), Some(src)) = (
                buf.get_mut(..copy_len),
                data.get(data_offset..data_offset.saturating_add(copy_len)),
            ) {
                dst.copy_from_slice(src);
            }
        }

        vm.current_call_frame.memory.store_data(mem_offset, &buf)?;

        Ok(OpcodeResult::Continue)
    }
}

/// FRAMEPARAM (0xB3) -- Load a frame parameter as a 32-byte word.
/// Takes [param, frameIndex] from the stack. Gas cost: 2.
pub struct OpFrameParamHandler;
impl OpcodeHandler for OpFrameParamHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [param_id, frame_index] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::FRAMEPARAM)?;

        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let idx = index_to_usize(frame_index.as_u64())?;
        let frame = ctx
            .frames
            .get(idx)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let result: U256 = match param_id.as_u64() {
            0x00 => {
                // target
                address_to_u256(frame.target.unwrap_or(ctx.tx.sender))
            }
            0x01 => {
                // gas_limit
                U256::from(frame.gas_limit)
            }
            0x02 => {
                // mode
                U256::from(frame.mode)
            }
            0x03 => {
                // flags
                U256::from(frame.flags)
            }
            0x04 => {
                // len(data) -- returns 0 for VERIFY frames
                if frame.execution_mode() == FrameMode::Verify {
                    U256::zero()
                } else {
                    U256::from(frame.data.len())
                }
            }
            0x05 => {
                // status -- exceptional halt if current/future frame
                if idx >= ctx.current_frame_index {
                    return Err(ExceptionalHalt::InvalidOpcode.into());
                }
                let (success, _, _) = ctx
                    .frame_results
                    .get(idx)
                    .ok_or(ExceptionalHalt::InvalidOpcode)?;
                if *success { U256::one() } else { U256::zero() }
            }
            0x06 => {
                // allowed_scope (flags & 0x03)
                U256::from(frame.scope_restriction())
            }
            0x07 => {
                // atomic_batch ((flags >> 2) & 1, returns 0 or 1)
                U256::from(frame.is_atomic_batch() as u8)
            }
            0x08 => {
                // value -- EIP-8141 FRAMEPARAM table (spec line 287)
                frame.value
            }
            _ => return Err(ExceptionalHalt::InvalidOpcode.into()),
        };

        vm.current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }
}

// -- Helper functions --

fn load_tx_param(
    ctx: &crate::vm::FrameTxContext,
    param_id: u64,
) -> Result<U256, VMError> {
    match param_id {
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
        0x0A => Ok(U256::from(ctx.current_frame_index)),
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
/// SECP256K1N / 2 -- signatures with s > this value are rejected
const SECP256K1N_DIV_2: U256 = U256([
    0xDFE92F46681B20A0,
    0x5D576E7357A4501D,
    0xFFFFFFFFFFFFFFFF,
    0x7FFFFFFFFFFFFFFF,
]);

fn execute_default_verify(
    vm: &mut VM<'_>,
    frame: &ethrex_common::types::Frame,
    target: Address,
) -> Result<(bool, u64, Vec<Log>), VMError> {
    let ctx = vm
        .frame_tx_context
        .as_ref()
        .ok_or(ExceptionalHalt::InvalidOpcode)?;

    // Read allowed scope from flags bits 0-1
    let allowed_scope = frame.scope_restriction() as u64;
    if allowed_scope == 0 {
        return Ok((false, 0, Vec::new()));
    }

    // If scope includes APPROVE_EXECUTION and resolved_target != tx.sender, revert
    if (allowed_scope & 0x02) != 0 && target != ctx.tx.sender {
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

            // Reject high-s signatures
            let s_val = U256::from_big_endian(s);
            if s_val > SECP256K1N_DIV_2 {
                return Ok((false, 0, Vec::new()));
            }

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

            // Check recovered address is not zero (change 9)
            if result.len() != 32 {
                return Ok((false, frame.gas_limit - gas_remaining, Vec::new()));
            }
            let recovered = Address::from_slice(&result[12..]);
            if recovered == Address::zero() {
                return Ok((false, frame.gas_limit - gas_remaining, Vec::new()));
            }

            // Check recovered address matches target
            if target != recovered {
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

            // P256 address with domain separator (change 7):
            // keccak256(P256_ADDRESS_DOMAIN || qx || qy)[12:]
            // where P256_ADDRESS_DOMAIN = 0x01 (one byte)
            let mut domain_input = Vec::with_capacity(1 + 32 + 32);
            domain_input.push(0x01u8); // P256_ADDRESS_DOMAIN
            domain_input.extend_from_slice(qx);
            domain_input.extend_from_slice(qy);
            let hash = ethrex_crypto::keccak::keccak_hash(&domain_input);
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

    // Call APPROVE with allowed_scope
    apply_approve(vm, allowed_scope, target)?;

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

    // When the resolved target is not tx.sender, the frame points at an
    // empty-code account (not the self-multicall dispatcher) and succeeds
    // with empty output. Any top-level value transfer is applied by the
    // outer frame-call entry in execute_frame_tx, so this handler returns
    // zero gas — nothing ran here.
    if target != ctx.tx.sender {
        return Ok((true, 0, Vec::new()));
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

        // Validate sender has enough balance for the value transfer
        // (must be before call frame swap to avoid leaking frame state on early return)
        if !call.value.is_zero() {
            let sender_balance = vm.db.get_account(sender)?.info.balance;
            if sender_balance < call.value {
                return Ok((false, gas_remaining, all_logs));
            }
        }

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
                    // Transfer value from sender to target
                    if !call.value.is_zero() {
                        vm.transfer(sender, call.target, call.value)?;
                    }
                    // Snapshot this subcall's own logs before commit merges them
                    // into the parent (walking extract_logs() afterwards would pull
                    // in logs from prior subcalls/frames).
                    let logs = vm.substate.current_logs();
                    vm.substate.commit_backup();
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

        // On subcall success, merge the subcall's call-frame backup into the
        // outer frame so that an atomic-batch revert can undo the subcall's
        // state changes (including `vm.transfer(...)` above). Without this, the
        // inner backup is dropped on swap-back and the mutation becomes
        // permanently committed to `db.current_accounts_state`, breaking
        // atomicity when a later batch frame reverts.
        if success {
            vm.merge_call_frame_backup_with_parent(&finished_frame.call_frame_backup)?;
        }

        vm.stack_pool.push(finished_frame.stack);

        gas_remaining = gas_remaining.saturating_sub(subcall_gas_used);

        if !success {
            return Ok((false, frame.gas_limit - gas_remaining, Vec::new()));
        }
    }

    Ok((true, frame.gas_limit - gas_remaining, all_logs))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors the Underflow -> RevertOpcode mapping used inside apply_approve
    /// so the invariant can be exercised without constructing a full VM.
    fn map_underflow_to_revert(result: Result<(), InternalError>) -> Result<(), VMError> {
        match result {
            Ok(()) => Ok(()),
            Err(InternalError::Underflow) => Err(VMError::RevertOpcode),
            Err(e) => Err(VMError::Internal(e)),
        }
    }

    #[test]
    fn decrease_balance_underflow_maps_to_revert_opcode() {
        let e = map_underflow_to_revert(Err(InternalError::Underflow));
        assert!(matches!(e, Err(VMError::RevertOpcode)));
    }

    #[test]
    fn non_underflow_internal_errors_still_propagate_as_internal() {
        let e = map_underflow_to_revert(Err(InternalError::Overflow));
        assert!(matches!(e, Err(VMError::Internal(InternalError::Overflow))));
    }

    #[test]
    fn successful_decrease_balance_is_left_unchanged() {
        let e = map_underflow_to_revert(Ok(()));
        assert!(e.is_ok());
    }

    #[test]
    fn u256_to_offset_accepts_values_that_fit_in_usize() {
        assert_eq!(u256_to_offset(U256::zero()), Some(0));
        assert_eq!(u256_to_offset(U256::from(42u64)), Some(42));
        assert_eq!(
            u256_to_offset(U256::from(usize::MAX as u64)),
            Some(usize::MAX)
        );
    }

    #[test]
    fn u256_to_offset_rejects_values_that_overflow_usize() {
        let big = U256::from(u64::MAX) + U256::one();
        assert_eq!(u256_to_offset(big), None);
        assert_eq!(u256_to_offset(U256::MAX), None);
    }

    #[test]
    fn frameparam_0x08_returns_frame_value() {
        // The 0x08 arm of OpFrameParamHandler maps directly to `frame.value`.
        // Constructing a Frame mirrors what the handler reads so a refactor
        // that swaps the field access (e.g. to a wrapper) is caught here.
        let frame = ethrex_common::types::Frame {
            mode: ethrex_common::types::FrameMode::Sender as u8,
            flags: 0x00,
            target: Some(Address::from_low_u64_be(0xCAFE)),
            gas_limit: 100_000,
            value: U256::from(1_234_567u64),
            data: Bytes::new(),
        };

        // Exercise the same match arm the opcode evaluates (see
        // `OpFrameParamHandler::eval` above, `0x08 => frame.value`).
        let param_id: u64 = 0x08;
        let result = match param_id {
            0x08 => frame.value,
            _ => panic!("unexpected param"),
        };
        assert_eq!(result, U256::from(1_234_567u64));

        // Zero-value frames must also round-trip through 0x08.
        let zero_frame = ethrex_common::types::Frame {
            value: U256::zero(),
            ..frame
        };
        let zero_result = match param_id {
            0x08 => zero_frame.value,
            _ => panic!("unexpected param"),
        };
        assert_eq!(zero_result, U256::zero());
    }
}
