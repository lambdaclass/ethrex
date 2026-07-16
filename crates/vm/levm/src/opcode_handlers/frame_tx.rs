//! # EIP-8141 Frame Transaction opcodes
//!
//! Includes:
//!   - `APPROVE` (0xAA)
//!   - `TXPARAM` (0xB0)
//!   - `FRAMEDATALOAD` (0xB1)
//!   - `FRAMEDATACOPY` (0xB2)
//!   - `FRAMEPARAM` (0xB3)
//!   - `SIGPARAM` (0xB4)
//!   - Default code for EOAs: `VERIFY` has the signature-check behavior;
//!     `SENDER` and `DEFAULT` return successfully as if calling empty code
//!     (pinned EIP-8141 spec §"Default code" lines 412-413).

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
pub(crate) fn index_to_usize(val: u64) -> Result<usize, VMError> {
    usize::try_from(val).map_err(|_| ExceptionalHalt::InvalidOpcode.into())
}

/// Convert a U256 offset to usize, returning None when the value does not fit
/// in usize on the current target. Used by FRAMEDATALOAD and FRAMEDATACOPY so
/// out-of-range offsets are treated as past-the-end rather than as an
/// exceptional halt (per the EIP-8141 spec the load returns zero and the copy
/// writes zero bytes).
pub fn u256_to_offset(value: U256) -> Option<usize> {
    if value.0[1] != 0 || value.0[2] != 0 || value.0[3] != 0 {
        return None;
    }
    usize::try_from(value.0[0]).ok()
}

/// Compute the transaction's MAXIMUM cost (spec line 387: APPROVE must
/// "collect the transaction's maximum cost from payer"):
/// `max_cost = max_fee_per_gas * total_gas_limit
///           + len(blob_hashes) * 131072 * max_fee_per_blob_gas`.
/// This is the single definition of "maximum cost": APPROVE (scopes 0x1/0x3)
/// debits it from the payer, TXPARAM(0x06) reports it (spec line 455), and the
/// mempool paymaster reservation reserves it. The end-of-tx refund returns
/// `max_cost - effective_gas_price * total_gas_used - base-rate blob burn`, so
/// the payer nets the effective-rate cost of the gas actually used plus the
/// EIP-4844 blob burn (intrinsic gas is inside `total_gas_used`, so it stays
/// non-refundable).
pub(crate) fn compute_tx_max_cost(ctx: &crate::vm::FrameTxContext) -> Result<U256, VMError> {
    let gas_cost = U256::from(ctx.tx.max_fee_per_gas)
        .checked_mul(U256::from(ctx.total_gas_limit))
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

/// Apply APPROVE side effects for the given scope.
/// This is shared between OpApproveHandler and (future) default code.
pub fn apply_approve(
    vm: &mut VM<'_>,
    scope: u64,
    frame_target: ethrex_common::Address,
) -> Result<(), VMError> {
    match scope {
        0x1 => {
            // APPROVE_PAYMENT: increment nonce, deduct max cost, record payer.
            // Per spec, the single transaction-scoped variable `payer` is
            // set on success; `payer.is_some()` is the source of truth for
            // "payment has been approved".
            let ctx = vm
                .frame_tx_context
                .as_ref()
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            if ctx.payer_address.is_some() {
                return Err(ExceptionalHalt::InvalidOpcode.into());
            }
            // EIP-8141: payment approval must not precede the sender's execution
            // approval. Per the spec's APPROVE_PAYMENT rules, revert the frame
            // while sender_approved == false (the sender authorizes execution
            // first; only then may a payer be bound and the max cost collected).
            if !ctx.sender_approved {
                return Err(VMError::RevertOpcode);
            }
            // EIP-8250: a payment approval's effects (nonce consumption, payer
            // recording, and the max-cost debit) must all survive together or
            // not at all. Inside an atomic batch a sibling frame's failure
            // rolls the whole batch's state back, which would unwind the
            // balance debit while the tx stayed authorized — minting the
            // difference at the end-of-tx refund. Rather than reconcile that
            // partial state (the spec's all-effects-durable rule is not yet
            // cross-client validated), forbid payment approval inside a batch:
            // reverting the frame leaves `payer` unset, and payment must be
            // granted from a non-batch frame (the validation prefix, which
            // already bans the batch flag). See docs/eip-8250.md.
            if ctx.tx.frame_is_in_atomic_batch(ctx.current_frame_index) {
                return Err(VMError::RevertOpcode);
            }
            let tx_cost = compute_tx_max_cost(ctx)?;
            let sender = ctx.tx.sender;

            vm.consume_keyed_nonces(sender)?;
            // Payer balance underflow is a frame-level revert, not a consensus
            // fault: the outer restore_cache_state() path rolls back the nonce
            // increment above when RevertOpcode propagates.
            match vm.decrease_account_balance(frame_target, tx_cost) {
                Ok(()) => {}
                Err(InternalError::Underflow) => return Err(VMError::RevertOpcode),
                Err(e) => return Err(VMError::Internal(e)),
            }

            let ctx = vm
                .frame_tx_context
                .as_mut()
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            ctx.payer_address = Some(frame_target);
        }
        0x2 => {
            // APPROVE_EXECUTION: set sender_approved (requires frame_target == tx.sender)
            let ctx = vm
                .frame_tx_context
                .as_ref()
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            if ctx.sender_approved {
                return Err(ExceptionalHalt::InvalidOpcode.into());
            }
            if frame_target != ctx.tx.sender {
                return Err(VMError::RevertOpcode);
            }
            let ctx = vm
                .frame_tx_context
                .as_mut()
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            ctx.sender_approved = true;
        }
        0x3 => {
            // APPROVE_EXECUTION_AND_PAYMENT: both, in one atomic step.
            let ctx = vm
                .frame_tx_context
                .as_ref()
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            if ctx.sender_approved || ctx.payer_address.is_some() {
                return Err(ExceptionalHalt::InvalidOpcode.into());
            }
            if frame_target != ctx.tx.sender {
                return Err(VMError::RevertOpcode);
            }
            // See scope 0x1: payment approval inside an atomic batch would let a
            // sibling revert unwind the balance debit while the tx stays
            // authorized — forbidden.
            if ctx.tx.frame_is_in_atomic_batch(ctx.current_frame_index) {
                return Err(VMError::RevertOpcode);
            }
            let tx_cost = compute_tx_max_cost(ctx)?;
            let sender = ctx.tx.sender;

            vm.consume_keyed_nonces(sender)?;
            // See scope 0x1 above for the Underflow → RevertOpcode rationale.
            match vm.decrease_account_balance(frame_target, tx_cost) {
                Ok(()) => {}
                Err(InternalError::Underflow) => return Err(VMError::RevertOpcode),
                Err(e) => return Err(VMError::Internal(e)),
            }

            let ctx = vm
                .frame_tx_context
                .as_mut()
                .ok_or(ExceptionalHalt::InvalidOpcode)?;
            ctx.sender_approved = true;
            ctx.payer_address = Some(frame_target);
        }
        _ => {
            // scope 0 and any other value are invalid
            return Err(ExceptionalHalt::InvalidOpcode.into());
        }
    }
    Ok(())
}

/// APPROVE (0xAA) -- Frame transaction approval opcode.
///
/// Pops [offset, length, scope] from the stack.
/// - scope 0x1 (APPROVE_PAYMENT): increment nonce, deduct tx cost, record payer
/// - scope 0x2 (APPROVE_EXECUTION): set sender_approved (requires resolved_target == tx.sender)
/// - scope 0x3 (APPROVE_EXECUTION_AND_PAYMENT): both, in one atomic step
/// - scope 0x0 (APPROVE_NONE) and any value > 3: invalid (exceptional halt)
///
/// The requested scope must also be a subset of the frame's allowed scope, taken
/// from flags bits 0-1 (`frame.scope_restriction()`). When the allowed scope is 0
/// (APPROVE_SCOPE_NONE) no approval may be granted in the frame at all, so APPROVE
/// halts (consistent with `execute_default_verify`).
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
            .tx
            .frames
            .get(ctx.current_frame_index)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        // EIP-7906: APPROVE is forbidden inside a POST_TX frame. A POST_TX frame
        // runs as a read-only assertion (dispatched as a STATICCALL); the spec
        // disallows all state manipulation there and explicitly forbids APPROVE.
        // Gate on the POST_TX mode specifically rather than on `is_static`: VERIFY
        // frames are also static, but APPROVE is precisely how they grant sender /
        // payer approval, so they must keep working.
        if current_frame.execution_mode() == FrameMode::PostTx {
            return Err(ExceptionalHalt::InvalidOpcode.into());
        }

        let frame_target = current_frame.target.unwrap_or(ctx.tx.sender);
        if vm.current_call_frame.to != frame_target {
            return Err(VMError::RevertOpcode);
        }

        // Enforce scope restriction from flags bits 0-1.
        // allowed_scope == 0 is APPROVE_SCOPE_NONE: no approval may be granted
        // in this frame at all (consistent with execute_default_verify).
        let allowed_scope = current_frame.scope_restriction();
        let scope_val = u64::try_from(scope).unwrap_or(u64::MAX);
        // requested scope must be a non-zero subset of a (necessarily non-zero) allowed_scope
        if scope_val == 0 || scope_val > 3 || (scope_val & u64::from(allowed_scope)) != scope_val {
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

        // Block-invariant knob flag (mirrors env.slot_number): gates the
        // resolved-payer index 0x11 so pre-knob blocks preserve its historical
        // exceptional-halt and re-execute identically.
        let payer_txparam_active = vm.env.config.payer_txparam_active;

        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let param_id = u64::try_from(param_id).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let result = load_tx_param(ctx, param_id, payer_txparam_active)?;
        vm.current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }
}

/// FRAMEDATALOAD (0xB1) -- Load one 32-byte word from a frame's data.
/// Stack: [offset, frameIndex] with offset on top (popped first); frameIndex is
/// the deeper operand. Gas cost: 3.
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

        let frame_index = u64::try_from(frame_index).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let idx = index_to_usize(frame_index)?;
        let frame = ctx
            .tx
            .frames
            .get(idx)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        // Out-of-usize offsets are past-the-end: the word stays zero-filled.
        let mut word = [0u8; 32];
        if let Some(byte_offset) = u256_to_offset(offset) {
            let data = &frame.data;
            let available = data.len().saturating_sub(byte_offset);
            let copy_len = available.min(32);
            if copy_len > 0
                && let Some(src) = data.get(byte_offset..byte_offset.saturating_add(copy_len))
            {
                // copy_len <= 32 == word.len(), so this slice is in bounds.
                if let Some(dst) = word.get_mut(..copy_len) {
                    dst.copy_from_slice(src);
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
/// Gas cost matches CALLDATACOPY.
pub struct OpFrameDataCopyHandler;
impl OpcodeHandler for OpFrameDataCopyHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [mem_offset, data_offset, length, frame_index] = *vm.current_call_frame.stack.pop()?;
        let (length, mem_offset) = size_offset_to_usize(length, mem_offset)?;
        // Out-of-usize data_offset is past-the-end: destination stays zero-filled.
        let data_offset_opt = u256_to_offset(data_offset);

        let new_memory_size = calculate_memory_size(mem_offset, length)?;
        let current_memory_size = vm.current_call_frame.memory.len();
        // Charging memory-expansion gas before the frame-context guard below is
        // intentional: the caller pays for the memory growth it requested even
        // when the opcode then halts for running outside a frame tx.
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::framedatacopy(
                new_memory_size,
                current_memory_size,
                length,
            )?)?;

        // Frame-context and frame_index checks precede the zero-length early
        // return: an out-of-bounds frameIndex halts exceptionally even when
        // length == 0 (EIP-8141 §FRAMEDATACOPY, consensus parity with FRAMEDATALOAD).
        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let frame_index = u64::try_from(frame_index).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let idx = index_to_usize(frame_index)?;
        let frame = ctx
            .tx
            .frames
            .get(idx)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        if length == 0 {
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
/// Stack: [param, frameIndex] with frameIndex on top (matches SIGPARAM). Gas cost: 2.
pub struct OpFrameParamHandler;
impl OpcodeHandler for OpFrameParamHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [frame_index, param_id] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::FRAMEPARAM)?;

        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let frame_index = u64::try_from(frame_index).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let idx = index_to_usize(frame_index)?;
        let frame = ctx
            .tx
            .frames
            .get(idx)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let param_id = u64::try_from(param_id).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let result: U256 = match param_id {
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
                // len(data)
                U256::from(frame.data.len())
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
                U256::from(u8::from(frame.is_atomic_batch()))
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

/// SIGPARAM (0xB4) -- signature-scoped metadata (EIP-8141, spec commit fe0940cae2).
/// Stack: [param, signatureIndex] with signatureIndex on top. Gas cost: 2.
/// Raw `signature` bytes are intentionally NOT exposed.
pub struct OpSigParamHandler;
impl OpcodeHandler for OpSigParamHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [signature_index, param] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::SIGPARAM)?;

        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let signature_index =
            u64::try_from(signature_index).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let idx = index_to_usize(signature_index)?;
        let sig = ctx
            .tx
            .signatures
            .get(idx)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let param = u64::try_from(param).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let result = match param {
            0x00 => address_to_u256(sig.signer), // effective signer
            0x01 => U256::from(sig.scheme),
            0x02 => {
                // msg: 0 when empty (canonical sig_hash case), else the 32-byte digest.
                if sig.msg.is_empty() {
                    U256::zero()
                } else {
                    U256::from_big_endian(&sig.msg)
                }
            }
            0x03 => U256::from(sig.signature.len()),
            _ => return Err(ExceptionalHalt::InvalidOpcode.into()),
        };
        vm.current_call_frame.stack.push(result)?;
        Ok(OpcodeResult::Continue)
    }
}

/// RECENTROOTREFLOAD (0xB5, EIP-8272) -- read a field of a declared recent-root
/// reference from the signed envelope. Stack: `[field, index]` with `field` on
/// top (popped first), `index` second. `field` 0 => source_id, 1 => slot,
/// 2 => root. Gas: 3. Reads only the envelope, never contract storage; allowed
/// in any frame mode (incl. VERIFY). Exceptional-halt if
/// `index >= len(recent_root_references)` or `field > 2`.
pub struct OpRecentRootRefLoadHandler;
impl OpcodeHandler for OpRecentRootRefLoadHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [field, index] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::RECENTROOTREFLOAD)?;

        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let index = u64::try_from(index).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let idx = index_to_usize(index)?;
        let reference = ctx
            .tx
            .recent_root_references
            .get(idx)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let field = u64::try_from(field).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let result = match field {
            0 => U256::from_big_endian(reference.source_id.as_bytes()),
            1 => U256::from(reference.slot),
            2 => U256::from_big_endian(reference.root.as_bytes()),
            _ => return Err(ExceptionalHalt::InvalidOpcode.into()),
        };
        vm.current_call_frame.stack.push(result)?;
        Ok(OpcodeResult::Continue)
    }
}

/// NONCEKEYLOAD (0xB9) -- read `nonce_keys[index]` from the signed envelope.
/// Stack: `[index]`. Gas: 3. Reads only the envelope, never contract storage;
/// allowed in any frame mode (incl. VERIFY). Exceptional-halt if
/// `index >= len(nonce_keys)`.
///
/// SPEC DIVERGENCE (EIP-8250): the spec deliberately exposes only `len`
/// (TXPARAM 0x0D), the whole-set keccak digest (0x0E) and `nonce_keys[0]`
/// (0x10) — there is no per-index accessor; the digest is its substitute. This
/// opcode is an ethrex-only extension for the Hegotá devnet (see
/// docs/eip-8250.md). Consensus-visible: its byte, gas, and out-of-range
/// (all-gas exceptional halt) semantics must match across clients.
pub struct OpNonceKeyLoadHandler;
impl OpcodeHandler for OpNonceKeyLoadHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [index] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::NONCEKEYLOAD)?;

        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let key = load_nonce_key(ctx, index)?;
        vm.current_call_frame.stack.push(key)?;
        Ok(OpcodeResult::Continue)
    }
}

// -- Helper functions --

pub fn load_tx_param(
    ctx: &crate::vm::FrameTxContext,
    param_id: u64,
    payer_txparam_active: bool,
) -> Result<U256, VMError> {
    match param_id {
        0x00 => Ok(U256::from(0x06u8)), // tx_type (EIP-8141 = type 6)
        0x01 => Ok(U256::from(ctx.tx.nonce_seq)),
        0x02 => Ok(address_to_u256(ctx.tx.sender)),
        0x03 => Ok(U256::from(ctx.tx.max_priority_fee_per_gas)),
        0x04 => Ok(U256::from(ctx.tx.max_fee_per_gas)),
        0x05 => Ok(ctx.tx.max_fee_per_blob_gas),
        0x06 => compute_tx_max_cost(ctx),
        0x07 => Ok(U256::from(ctx.tx.blob_versioned_hashes.len())),
        0x08 => {
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(ctx.sig_hash.as_bytes());
            Ok(U256::from_big_endian(&bytes))
        }
        0x09 => Ok(U256::from(ctx.tx.frames.len())),
        0x0A => Ok(U256::from(ctx.current_frame_index)),
        0x0B => Ok(U256::from(ctx.tx.signatures.len())),
        // EIP-8250 keyed nonces.
        0x0C => Ok(U256::from(ctx.legacy_sender_nonce)),
        0x0D => Ok(U256::from(ctx.tx.nonce_keys.len())),
        0x0E => Ok(U256::from_big_endian(ctx.tx.nonce_keys_hash().as_bytes())),
        // 0x10 = nonce_keys[0], relocated from the spec's 0x0B (ethrex keeps 0x0B
        // for len(signatures); divergence documented in docs/eip-8250.md).
        0x10 => ctx
            .tx
            .nonce_keys
            .first()
            .copied()
            .ok_or(ExceptionalHalt::InvalidOpcode.into()),
        // EIP-8272: count of recent-root references.
        0x0F => Ok(U256::from(ctx.tx.recent_root_references.len())),
        // Resolved payer address (ethrex extension, not in the EIP-8141 draft).
        // Gated on the payer_txparam knob: before it (and on chains without it)
        // this index falls through to the exceptional halt below, so already-
        // produced blocks re-execute identically. When active it returns the
        // account a payment-scoped APPROVE charged, zero-padded like the 0x02
        // sender. `None` (payer not yet resolved — e.g. a validation-prefix
        // VERIFY frame that runs before payment) reads as the zero address,
        // matching the receipt's payer encoding; a committed tx always has a
        // resolved payer (post-execution invariant), so the SENDER/POST_TX
        // frames that consume this always observe the real payer.
        0x11 if payer_txparam_active => Ok(ctx
            .payer_address
            .map(address_to_u256)
            .unwrap_or_else(U256::zero)),
        _ => Err(ExceptionalHalt::InvalidOpcode.into()),
    }
}

/// Read `nonce_keys[index]` from the signed envelope, or exceptional-halt
/// (`InvalidOpcode`, consuming all gas) if `index` is out of range or does not
/// fit in a usize. Backs the NONCEKEYLOAD opcode; kept as a pure helper so the
/// bounds behavior is unit-testable without a full VM.
pub(crate) fn load_nonce_key(
    ctx: &crate::vm::FrameTxContext,
    index: U256,
) -> Result<U256, VMError> {
    let index = u64::try_from(index).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
    let idx = index_to_usize(index)?;
    ctx.tx
        .nonce_keys
        .get(idx)
        .copied()
        .ok_or(ExceptionalHalt::InvalidOpcode.into())
}

pub fn address_to_u256(addr: ethrex_common::Address) -> U256 {
    let mut bytes = [0u8; 32];
    bytes[12..].copy_from_slice(addr.as_bytes());
    U256::from_big_endian(&bytes)
}

// -- Default code for EOAs (EIP-8141) --

/// Execute default code for an EOA target in a frame transaction.
///
/// When a frame targets an address with no deployed code (an EOA), the protocol
/// runs built-in "default code" instead of executing a normal CALL. `VERIFY`
/// runs the signature-check logic; `SENDER` and `DEFAULT` return successfully
/// as if calling empty code (pinned EIP-8141 spec §"Default code" lines 412-413).
///
/// Returns `(success, gas_used, logs)`.
pub fn execute_default_code(
    vm: &mut VM<'_>,
    frame: &ethrex_common::types::Frame,
    target: Address,
) -> Result<(bool, u64, Vec<Log>), VMError> {
    // EIP-8272: RECENT_ROOT_ADDRESS carries no runtime bytecode (the write is
    // native, docs/eip-8272.md divergence #4), so a frame targeting it lands
    // in this empty-code path and executes the recent-root write instead of
    // the generic default code.
    if target == ethrex_common::types::frame_tx_recent_root() {
        return execute_recent_root_frame(vm, frame);
    }
    match frame.execution_mode() {
        FrameMode::Verify => execute_default_verify(vm, frame, target),
        // Pinned EIP-8141 spec (fe0940cae2) §"Default code" lines 412-413:
        // a SENDER or DEFAULT frame whose target has no code "returns
        // successfully as if calling empty code" — this is what makes a plain
        // ETH transfer to an EOA work (spec §EOA support / Example 1).
        // Consumes no execution gas (the frame's value transfer is handled by
        // the caller's deferred transfer).
        // EIP-7906: POST_TX default-code is handled like SENDER/DEFAULT.
        FrameMode::Sender | FrameMode::Default | FrameMode::PostTx => Ok((true, 0, Vec::new())),
    }
}

/// EIP-8272 write semantics for a frame whose target is RECENT_ROOT_ADDRESS,
/// mirroring what real predeploy bytecode would observe: msg.sender is the
/// frame's caller (ENTRY_POINT for DEFAULT frames, the tx sender for SENDER
/// frames), VERIFY/POST_TX frames run statically so the write fails, and the
/// call reverts unless the frame data is exactly 64 bytes (`salt ‖ root`)
/// with zero value. A successful write costs `RECENT_ROOT_WRITE_GAS`.
fn execute_recent_root_frame(
    vm: &mut VM<'_>,
    frame: &ethrex_common::types::Frame,
) -> Result<(bool, u64, Vec<Log>), VMError> {
    // VERIFY and POST_TX frames are dispatched as static calls in the frame
    // loop; the write is a state change, so it must fail there.
    let is_static = matches!(
        frame.execution_mode(),
        FrameMode::Verify | FrameMode::PostTx
    );
    if is_static || frame.data.len() != 64 || !frame.value.is_zero() {
        return Ok((false, 0, Vec::new()));
    }
    if frame.gas_limit < gas_cost::RECENT_ROOT_WRITE_GAS {
        // The write out-of-gasses: the frame consumes its whole budget.
        return Ok((false, frame.gas_limit, Vec::new()));
    }
    let ctx = vm
        .frame_tx_context
        .as_ref()
        .ok_or(ExceptionalHalt::InvalidOpcode)?;
    let caller = match frame.execution_mode() {
        FrameMode::Sender => ctx.tx.sender,
        _ => ethrex_common::types::frame_tx_entry_point(),
    };
    let salt = frame.data.get(..32).ok_or(ExceptionalHalt::OutOfBounds)?;
    let root = frame
        .data
        .get(32..64)
        .ok_or(ExceptionalHalt::OutOfBounds)?;
    vm.recent_root_native_write(caller, salt, root)?;
    Ok((true, gas_cost::RECENT_ROOT_WRITE_GAS, Vec::new()))
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
    let allowed_scope = u64::from(frame.scope_restriction());
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
mod max_cost_tests {
    use super::{address_to_u256, compute_tx_max_cost, load_nonce_key, load_tx_param};
    use crate::errors::{ExceptionalHalt, VMError};
    use crate::vm::FrameTxContext;
    use ethrex_common::{Address, H256, U256, types::FrameTransaction};

    fn ctx(max_fee: u64, blobs: usize, max_blob_fee: u64, total_gas_limit: u64) -> FrameTxContext {
        let tx = FrameTransaction {
            max_fee_per_gas: max_fee,
            max_fee_per_blob_gas: U256::from(max_blob_fee),
            blob_versioned_hashes: vec![H256::zero(); blobs],
            ..Default::default()
        };
        FrameTxContext {
            sender_approved: false,
            payer_address: None,
            frame_results: Vec::new(),
            current_frame_index: 0,
            sig_hash: H256::zero(),
            tx,
            approve_called_in_current_frame: false,
            total_gas_limit,
            legacy_sender_nonce: 0,
        }
    }

    #[test]
    fn max_cost_is_max_fee_times_limit_plus_max_blob_cost() {
        // 10 * 100_000 + 2 * 131072 * 5 = 1_000_000 + 1_310_720
        let c = ctx(10, 2, 5, 100_000);
        assert_eq!(compute_tx_max_cost(&c).unwrap(), U256::from(2_310_720u64));
        // No blobs: just max_fee * total_gas_limit.
        let c = ctx(7, 0, 999, 21_000);
        assert_eq!(compute_tx_max_cost(&c).unwrap(), U256::from(147_000u64));
    }

    #[test]
    fn txparam_0x06_reports_the_same_maximum_cost_approve_debits() {
        // TXPARAM(0x06) and the APPROVE debit must stay one definition of
        // "maximum cost"; a split between them is a consensus bug.
        let c = ctx(10, 2, 5, 100_000);
        assert_eq!(
            load_tx_param(&c, 0x06, false).unwrap(),
            compute_tx_max_cost(&c).unwrap()
        );
    }

    #[test]
    fn txparam_0x11_reads_resolved_payer_when_knob_active() {
        let payer = Address::from_low_u64_be(0xABCD);
        let mut c = ctx(10, 0, 0, 21_000);
        c.payer_address = Some(payer);
        assert_eq!(
            load_tx_param(&c, 0x11, true).unwrap(),
            address_to_u256(payer),
            "0x11 must report the resolved payer when the knob is active"
        );
    }

    #[test]
    fn txparam_0x11_reads_zero_before_payer_resolved() {
        // A validation-prefix VERIFY frame runs before payment is approved.
        let c = ctx(10, 0, 0, 21_000);
        assert!(c.payer_address.is_none());
        assert_eq!(
            load_tx_param(&c, 0x11, true).unwrap(),
            U256::zero(),
            "0x11 must read the zero address before the payer is resolved"
        );
    }

    #[test]
    fn txparam_0x11_halts_when_knob_inactive() {
        // History preservation: before the payer_txparam knob, 0x11 keeps its
        // exceptional halt so already-produced blocks re-execute identically —
        // even when a payer is present.
        let mut c = ctx(10, 0, 0, 21_000);
        c.payer_address = Some(Address::from_low_u64_be(0xABCD));
        assert!(matches!(
            load_tx_param(&c, 0x11, false),
            Err(VMError::ExceptionalHalt(ExceptionalHalt::InvalidOpcode))
        ));
    }

    #[test]
    fn txparam_unknown_index_halts_even_when_knob_active() {
        let c = ctx(10, 0, 0, 21_000);
        assert!(matches!(
            load_tx_param(&c, 0x12, true),
            Err(VMError::ExceptionalHalt(ExceptionalHalt::InvalidOpcode))
        ));
    }

    #[test]
    fn nonce_key_load_reads_by_index_and_halts_out_of_range() {
        let mut c = ctx(10, 0, 0, 21_000);
        c.tx.nonce_keys = vec![U256::from(0u64), U256::from(5u64), U256::from(9u64)];
        // In-range indices return the element.
        assert_eq!(load_nonce_key(&c, U256::zero()).unwrap(), U256::from(0u64));
        assert_eq!(load_nonce_key(&c, U256::one()).unwrap(), U256::from(5u64));
        assert_eq!(
            load_nonce_key(&c, U256::from(2u64)).unwrap(),
            U256::from(9u64)
        );
        // Out-of-range and oversized indices exceptional-halt (never a silent
        // zero-fill — a differing value would fork consensus).
        assert!(matches!(
            load_nonce_key(&c, U256::from(3u64)),
            Err(VMError::ExceptionalHalt(ExceptionalHalt::InvalidOpcode))
        ));
        assert!(matches!(
            load_nonce_key(&c, U256::MAX),
            Err(VMError::ExceptionalHalt(ExceptionalHalt::InvalidOpcode))
        ));
    }
}
