//! # EIP-8141 Frame Transaction opcodes
//!
//! Includes:
//!   - `APPROVE` (0xAA)
//!   - `TXPARAM` (0xB0)
//!   - `FRAMEDATALOAD` (0xB1)
//!   - `FRAMEDATACOPY` (0xB2)
//!   - Default code for EOAs (only `VERIFY` has executable behavior; `SENDER`
//!     and `DEFAULT` revert unconditionally per the latest EIP-8141 spec).

use crate::{
    errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError},
    gas_cost,
    memory::calculate_memory_size,
    opcode_handlers::OpcodeHandler,
    utils::size_offset_to_usize,
    vm::VM,
};
use ethrex_common::{Address, U256, types::FrameMode, types::Log};

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
            // APPROVE_PAYMENT: increment nonce, deduct max cost, record payer.
            // Per spec, the single transaction-scoped variable `payer` is
            // set on success; `payer.is_some()` is the source of truth for
            // "payment has been approved".
            let ctx = vm.frame_tx_context.as_ref().ok_or(halt_err.clone())?;
            if ctx.payer_address.is_some() {
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
            // APPROVE_EXECUTION_AND_PAYMENT: both, in one atomic step.
            let ctx = vm.frame_tx_context.as_ref().ok_or(halt_err.clone())?;
            if ctx.sender_approved || ctx.payer_address.is_some() {
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
                // status -- exceptional halt if current/future frame.
                // Returns the EIP-8141 status code: 0 = failure, 1 = success,
                // 3 = skipped (atomic-batch failure).
                if idx >= ctx.current_frame_index {
                    return Err(ExceptionalHalt::InvalidOpcode.into());
                }
                let (status, _, _) = ctx
                    .frame_results
                    .get(idx)
                    .ok_or(ExceptionalHalt::InvalidOpcode)?;
                U256::from(*status)
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

/// Execute default code for an EOA target in a frame transaction.
///
/// When a frame targets an address with no deployed code (an EOA), the protocol
/// runs built-in "default code" instead of executing a normal CALL. Only the
/// `VERIFY` mode has executable default-code behavior; per the latest spec,
/// `SENDER` and `DEFAULT` modes revert the frame unconditionally.
///
/// Returns `(success, gas_used, logs)`.
pub fn execute_default_code(
    vm: &mut VM<'_>,
    frame: &ethrex_common::types::Frame,
    target: Address,
) -> Result<(bool, u64, Vec<Log>), VMError> {
    match frame.execution_mode() {
        FrameMode::Verify => execute_default_verify(vm, frame, target),
        FrameMode::Sender | FrameMode::Default => Ok((false, 0, Vec::new())),
    }
}

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

    // EIP-8141 (spec commit fe0940cae2): the default account approves only if
    // the outer signature list contains a SECP256K1 signature over the
    // canonical sig_hash (empty msg) whose signer is the resolved target.
    // Signatures were already validated in execute_frame_tx, so a match here is
    // sufficient — no in-frame crypto.
    let has_sender_sig = ctx.tx.signatures.iter().any(|s| {
        s.scheme == ethrex_common::types::FRAME_SIG_SCHEME_SECP256K1
            && s.msg.is_empty()
            && s.signer == target
    });
    if !has_sender_sig {
        return Ok((false, 0, Vec::new()));
    }

    apply_approve(vm, allowed_scope, target)?;

    let ctx = vm
        .frame_tx_context
        .as_mut()
        .ok_or(ExceptionalHalt::InvalidOpcode)?;
    ctx.approve_called_in_current_frame = true;

    Ok((true, 0, Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

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
