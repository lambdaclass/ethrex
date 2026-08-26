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
//!     (EIP-8141 §"Default code").

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

/// Compute the transaction's MAXIMUM cost (EIP-8141 §Gas Accounting: APPROVE must
/// "collect the transaction's maximum cost from payer"):
/// `max_cost = max_fee_per_gas * total_gas_limit
///           + len(blob_hashes) * 131072 * max_fee_per_blob_gas`.
/// This is the single definition of "maximum cost": APPROVE (scopes 0x1/0x3)
/// debits it from the payer, TXPARAM(0x06) reports it, and the
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
        .checked_mul(ctx.blob_base_fee)
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
            //
            // EIP-8312 waives exactly this precondition for a vault-sender
            // transaction. The vault's code never calls APPROVE, so it can never
            // grant execution approval — without the waiver a sponsor could never
            // pay for a spend and the sponsored vault-sender form (the one that
            // lets one sponsor serve many concurrent spends) would be unusable.
            // Nothing else is waived: SENDER frames stay invalid, so waiving this
            // does not let the vault act.
            let vault_sender = vm.env.config.utxo_frames_active
                && ctx.tx.sender == ethrex_common::types::utxo_vault();
            if !ctx.sender_approved && !vault_sender {
                return Err(VMError::RevertOpcode);
            }
            // EIP-8250: a payment approval's effects (nonce consumption, payer
            // recording, and the balance debit) must all survive together or
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

            // The payer is a protocol-touched account: collecting `max_cost`
            // adds it to `accessed_addresses` without charging a cold access,
            // as for `tx.sender` and the coinbase.
            vm.substate.add_accessed_address(frame_target);

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
            // Payment approval inside an atomic batch would let a sibling revert
            // unwind the balance debit while the tx stays authorized — forbidden
            // (EIP-8250 durability).
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

            // See scope 0x1: collecting `max_cost` warms the payer.
            vm.substate.add_accessed_address(frame_target);

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
/// TXPARAM index of the sender's legacy account nonce (EIP-8250).
/// EIP-8250's legacy account-nonce read, relocated from `0x0C` to `0x12`.
///
/// EIP-8141 v2 assigns `0x0C` to `state_gas_left`, and the spec's id wins. `0x11` was not
/// available either -- it is the resolved-payer read -- so this lands on the next free id.
/// Precedent for relocating an EIP-8250 id is in `docs/eip-8250.md`, which records the
/// earlier `0x0B -> 0x10` move for `nonce_keys[0]` after `0x0B` collided.
const TXPARAM_LEGACY_SENDER_NONCE: u64 = 0x12;

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
        // EIP-8250 §Mempool: a validation prefix that reads the sender's legacy
        // account nonce depends on it, so the mempool must revalidate when that
        // nonce changes and must not treat the transaction as replay-independent
        // of the sender's other keyed transactions.
        if vm.validation_observer.active && param_id == TXPARAM_LEGACY_SENDER_NONCE {
            vm.validation_observer.read_legacy_nonce = true;
        }
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
                // 2 = skipped (atomic-batch failure).
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
                // value -- EIP-8141 FRAMEPARAM table
                frame.value
            }
            _ => return Err(ExceptionalHalt::InvalidOpcode.into()),
        };

        vm.current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }
}

/// SIGPARAM (0xB4) -- signature-scoped metadata (EIP-8141).
/// Stack `[param, signatureIndex]` with `signatureIndex` on top; gas 2; returns one
/// word (0x00 effective signer, 0x01 scheme, 0x02 msg, 0x03 len(signature)).
///
/// EIP-8141 v2 moved the copy operation out of here into [`OpSigDataCopyHandler`],
/// so `param` 0x04 is no longer defined and halts like any other unknown param.
pub struct OpSigParamHandler;
impl OpcodeHandler for OpSigParamHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [signature_index, param] = *vm.current_call_frame.stack.pop()?;
        let param = u64::try_from(param).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let signature_index =
            u64::try_from(signature_index).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let idx = index_to_usize(signature_index)?;

        // Fixed gas, returns one word.
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::SIGPARAM)?;
        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;
        let sig = ctx
            .tx
            .signatures
            .get(idx)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;
        let result = match param {
            // Resolved signer: an absent signer resolves to tx.sender. EIP-8141
            // assigns no resolved signer to ARBITRARY entries, so asking for one
            // is an exceptional halt.
            0x00 => {
                if sig.scheme == ethrex_common::types::FRAME_SIG_SCHEME_ARBITRARY {
                    return Err(ExceptionalHalt::InvalidOpcode.into());
                }
                address_to_u256(sig.signer.unwrap_or(ctx.tx.sender))
            }
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
/// SIGDATACOPY (0xB5) -- copy an ARBITRARY signature's raw bytes into memory (EIP-8141 v2).
///
/// v2 split this out of `SIGPARAM(0x04)` and gave it its own byte. Stack, top first:
/// `memOffset`, `dataOffset`, `length`, `signatureIndex` -- CALLDATACOPY's operand order
/// with the signature index beneath it, and note that the index moved from the *top* of
/// the stack (where SIGPARAM took it) to the *bottom* of the four operands.
///
/// Gas is CALLDATACOPY's: the fixed 3, the per-word copy cost, and memory expansion.
/// Raw signature bytes of protocol-validated schemes stay un-introspectable so future
/// aggregation remains possible, so an out-of-range index or any scheme other than
/// ARBITRARY is an exceptional halt. Bytes past the end of the signature read as zero.
pub struct OpSigDataCopyHandler;
impl OpcodeHandler for OpSigDataCopyHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [mem_offset, data_offset, length, signature_index] =
            *vm.current_call_frame.stack.pop()?;
        let signature_index =
            u64::try_from(signature_index).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let idx = index_to_usize(signature_index)?;
        let (length, mem_offset) = size_offset_to_usize(length, mem_offset)?;
        let data_offset_opt = u256_to_offset(data_offset);

        let new_memory_size = calculate_memory_size(mem_offset, length)?;
        let current_memory_size = vm.current_call_frame.memory.len();
        // Charge memory-expansion gas before the context and scheme guards: the caller
        // pays for the growth it asked for even when the opcode goes on to halt.
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::framedatacopy(
                new_memory_size,
                current_memory_size,
                length,
            )?)?;

        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;
        let sig = ctx
            .tx
            .signatures
            .get(idx)
            .ok_or(ExceptionalHalt::InvalidOpcode)?;
        if sig.scheme != ethrex_common::types::FRAME_SIG_SCHEME_ARBITRARY {
            return Err(ExceptionalHalt::InvalidOpcode.into());
        }
        if length == 0 {
            return Ok(OpcodeResult::Continue);
        }
        let data = &sig.signature;
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
        // 0x0C is reserved for EIP-8141 v2's `state_gas_left`, which needs the per-frame
        // state pool to exist before it can report anything true. Until then it is not
        // implemented and falls through to the exceptional halt below, rather than
        // returning a number that would be wrong.
        0x12 => Ok(U256::from(ctx.legacy_sender_nonce)),
        0x0D => Ok(U256::from(ctx.tx.nonce_keys.len())),
        0x0E => Ok(U256::from_big_endian(ctx.tx.nonce_keys_hash().as_bytes())),
        // EIP-8272: count of recent-root references.
        0x0F => Ok(U256::from(ctx.tx.recent_root_references.len())),
        // 0x10 = nonce_keys[0], relocated from the spec's 0x0B (ethrex keeps 0x0B
        // for len(signatures); divergence documented in docs/eip-8250.md).
        0x10 => ctx
            .tx
            .nonce_keys
            .first()
            .copied()
            .ok_or(ExceptionalHalt::InvalidOpcode.into()),
        // Resolved payer address (ethrex extension, not in the EIP-8141 draft).
        // Gated on the payer_txparam knob: before it (and on chains without it)
        // this index falls through to the exceptional halt below, so already-
        // produced blocks re-execute identically. When active it returns the
        // account a payment-scoped APPROVE charged, zero-padded like the 0x02
        // sender. `None` (payer not yet resolved — e.g. a validation-prefix
        // VERIFY frame that runs before payment) reads as the zero address,
        // matching the receipt's payer encoding; a committed tx always has a
        // resolved payer (post-execution invariant), so the frames that run
        // after the validation prefix always observe the real payer.
        0x11 if payer_txparam_active => Ok(ctx
            .payer_address
            .map(address_to_u256)
            .unwrap_or_else(U256::zero)),
        _ => Err(ExceptionalHalt::InvalidOpcode.into()),
    }
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
/// as if calling empty code (EIP-8141 §"Default code").
///
/// Returns `(success, gas_used, logs)`.
pub fn execute_default_code(
    vm: &mut VM<'_>,
    frame: &ethrex_common::types::Frame,
    target: Address,
) -> Result<(bool, u64, Vec<Log>), VMError> {
    match frame.execution_mode() {
        Some(FrameMode::Verify) => execute_default_verify(vm, frame, target),
        // EIP-8141 §"Default code": a SENDER or DEFAULT frame whose target has no code "returns
        // successfully as if calling empty code" — this is what makes a plain
        // ETH transfer to an EOA work (spec §EOA support / Example 1).
        // Consumes no execution gas (the frame's value transfer is handled by
        // the caller's deferred transfer).
        Some(FrameMode::Sender | FrameMode::Default) => Ok((true, 0, Vec::new())),
        // A UTXO frame never reaches the default-code path: it executes no EVM
        // code and is dispatched natively by the frame loop before any target
        // code resolution. A reserved mode is rejected by static validation.
        Some(FrameMode::Utxo) | None => Err(ExceptionalHalt::InvalidOpcode.into()),
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
    let allowed_scope = u64::from(frame.scope_restriction());
    if allowed_scope == 0 {
        return Ok((false, 0, Vec::new()));
    }

    // If scope includes APPROVE_EXECUTION and resolved_target != tx.sender, revert
    if (allowed_scope & 0x02) != 0 && target != ctx.tx.sender {
        return Ok((false, 0, Vec::new()));
    }

    // EIP-8141: the default account approves only if the signature at a specific
    // index — 0 when the allowed scope includes APPROVE_EXECUTION, else 1 (the
    // payment-only case, where index 0 belongs to the sender's own verify frame)
    // — is a SECP256K1 signature over the canonical sig_hash (empty msg) whose
    // resolved signer is the resolved target. An absent signer resolves to
    // tx.sender. Signatures were already validated in execute_frame_tx, so a
    // match here is sufficient — no in-frame crypto.
    let sig_index = if (allowed_scope & 0x02) != 0 { 0 } else { 1 };
    let sender_sig_ok = ctx.tx.signatures.get(sig_index).is_some_and(|s| {
        s.scheme == ethrex_common::types::FRAME_SIG_SCHEME_SECP256K1
            && s.msg.is_empty()
            && s.signer.unwrap_or(ctx.tx.sender) == target
    });
    if !sender_sig_ok {
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
    use super::{address_to_u256, compute_tx_max_cost, load_tx_param};
    use crate::errors::{ExceptionalHalt, VMError};
    use crate::vm::FrameTxContext;
    use ethrex_common::{Address, H256, U256, types::FrameTransaction};

    fn ctx(max_fee: u64, blobs: usize, blob_base_fee: u64, total_gas_limit: u64) -> FrameTxContext {
        let tx = FrameTransaction {
            max_fee_per_gas: max_fee,
            // Deliberately far above the base fee: `max_fee_per_blob_gas` bounds
            // inclusion only and must not reach `max_cost`.
            max_fee_per_blob_gas: U256::from(blob_base_fee).saturating_mul(U256::from(1_000u64)),
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
            blob_base_fee: U256::from(blob_base_fee),
        }
    }

    #[test]
    fn max_cost_is_max_fee_times_limit_plus_base_rate_blob_cost() {
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
}

/// A UTXO frame's settled payout, produced by [`execute_utxo_frame`] and applied
/// after the frame loop (EIP-8312 §Settlement).
#[derive(Debug, Clone)]
pub struct UtxoSettlement {
    /// Index of the frame that produced this settlement, so its logs can be
    /// attributed to the right receipt entry.
    pub frame_index: usize,
    /// `spend.actors[0]` when there is exactly one actor, else the vault: the
    /// `source` recorded in the outputs' `UtxoCreated` logs.
    pub source: Address,
    /// Total proven input value.
    pub spent_value: U256,
    /// Sum of the signed (non-change) output values.
    pub signed_out: U256,
    /// Index into `outputs` designating the change entry.
    pub change_index: usize,
    /// UTXO outputs (created as new UTXOs), in order.
    pub utxo_outs: Vec<(Address, U256)>,
    /// Account outputs (credited directly), in order.
    pub account_outs: Vec<(Address, U256)>,
    /// Whether this frame's spend is self-funded, i.e. the vault fronts the
    /// transaction's cost and the actual fee is deducted from the change.
    pub self_funded: bool,
    /// `GAS_NEW_ACCOUNT_STATE` reserves charged for account outputs, to be
    /// returned at settlement for recipients that already exist.
    pub new_account_reserve_each: u64,
}

/// EIP-8312 UTXO frame execution.
///
/// Executes no EVM code. Verifies each input's opening against the vault's
/// openings roots, atomically checks-and-sets its spent bit through the durable
/// tier, enforces value conservation including the transaction's maximum cost for
/// a self-funded spend, and assigns or binds the payer.
///
/// Returns the gas consumed plus the settlement to apply after the frame loop.
/// Any failed check makes the whole transaction invalid (as with a reverting
/// VERIFY frame), signalled by `Ok(None)`; the caller sets `tx_invalid`.
pub fn execute_utxo_frame(
    vm: &mut VM<'_>,
    frame: &ethrex_common::types::Frame,
    frame_index: usize,
) -> Result<Option<(u64, UtxoSettlement)>, VMError> {
    use ethrex_common::types::{
        BATCH_SIZE, RING_SIZE, Spend, batch_slot_for_block, fold, is_spent, opening_leaf,
        ring_slot, spent_bit_location, utxo_vault,
    };

    // Static validation already decoded and bounds-checked this payload; decode
    // again here rather than threading it through, so execution never depends on
    // a value computed outside consensus.
    let Ok(spend) = Spend::decode_frame_data(&frame.data) else {
        return Ok(None);
    };

    // --- Gas ---------------------------------------------------------------
    // Regular and state components are summed into one frame charge; the state
    // components are tracked separately so block-level 2D accounting can split
    // them back out, and so the durable ones survive scope reverts.
    let state_per_spent_bit = spent_bit_state_gas(vm);
    let reserve_per_account_out = vm.state_gas_new_account;

    let mut sibling_count: u64 = 0;
    for input in &spend.inputs {
        sibling_count = sibling_count
            .saturating_add(u64::try_from(input.siblings.len()).unwrap_or(u64::MAX))
            .saturating_add(u64::try_from(input.batch_siblings.len()).unwrap_or(u64::MAX));
    }
    let inputs = u64::try_from(spend.inputs.len()).unwrap_or(u64::MAX);
    let utxo_out_count = u64::try_from(spend.utxo_outs.len()).unwrap_or(u64::MAX);
    let account_out_count = u64::try_from(spend.account_outs.len()).unwrap_or(u64::MAX);

    let regular_gas = gas_cost::GAS_UTXO_FRAME
        .saturating_add(gas_cost::GAS_UTXO_INPUT.saturating_mul(inputs))
        .saturating_add(gas_cost::GAS_UTXO_SIBLING.saturating_mul(sibling_count))
        .saturating_add(gas_cost::GAS_UTXO_OUT.saturating_mul(utxo_out_count))
        .saturating_add(gas_cost::GAS_UTXO_ACCOUNT_OUT.saturating_mul(account_out_count));
    let state_gas = state_per_spent_bit
        .saturating_mul(inputs)
        .saturating_add(reserve_per_account_out.saturating_mul(account_out_count));
    let utxo_frame_gas = regular_gas.saturating_add(state_gas);

    // Per the EIP an under-provisioned UTXO frame invalidates the transaction —
    // it is not a failed frame that the transaction survives.
    if frame.gas_limit < utxo_frame_gas {
        return Ok(None);
    }

    // --- Fee caps ----------------------------------------------------------
    // The actors signed caps; the envelope (which for a vault-sender transaction
    // nobody signed) must stay within them.
    let ctx = vm
        .frame_tx_context
        .as_ref()
        .ok_or(ExceptionalHalt::InvalidOpcode)?;
    let tx_max_fee = U256::from(ctx.tx.max_fee_per_gas);
    let tx_max_priority = U256::from(ctx.tx.max_priority_fee_per_gas);
    let tx_gas_limit = ctx.total_gas_limit;
    let max_cost = compute_tx_max_cost(ctx)?;
    let tx_sender = ctx.tx.sender;
    let chain_id = ctx.tx.chain_id;

    if tx_max_fee > spend.max_fee_per_gas
        || tx_max_priority > spend.max_priority_fee_per_gas
        || tx_gas_limit > spend.max_gas_limit
    {
        return Ok(None);
    }

    // --- Inputs: verify openings and set spent bits -------------------------
    let block_number = vm.env.block_number;
    let mut spent_value = U256::zero();
    for input in &spend.inputs {
        // The recipient named by the proven opening must be one of the actors
        // that signed this spend.
        if !spend.actors.contains(&input.recipient) {
            return Ok(None);
        }

        let leaf = opening_leaf(input.index, input.source, input.recipient, input.value);
        let mut root = fold(leaf, input.position, &input.siblings);

        if input.batch_siblings.is_empty() {
            // Ring proof: the creation block's openings root must still be in the
            // ring, and a UTXO is only spendable from the block AFTER its
            // creation (its root is written at the creation block's end).
            let age = block_number.checked_sub(input.creation_block);
            // `age == 0` (created in this very block) and a creation block ahead of
            // us are TRANSIENT: the openings root is written at the creation
            // block's end, so the spend becomes valid in a later block. Surface
            // that distinctly so the builder keeps the transaction pooled instead
            // of evicting a valid spend.
            let Some(age) = age else {
                return Err(VMError::TxValidation(
                    crate::errors::TxValidationError::UtxoNotYetSpendable,
                ));
            };
            if age == 0 {
                return Err(VMError::TxValidation(
                    crate::errors::TxValidationError::UtxoNotYetSpendable,
                ));
            }
            if age > RING_SIZE {
                return Ok(None); // aged out of the ring: needs a batch proof
            }
            let slot = u256_to_h256(ring_slot(input.creation_block));
            if vm.read_vault_slot(slot)? != U256::from_big_endian(root.as_bytes()) {
                return Ok(None);
            }
        } else {
            // Batch proof: the batch containing the creation block must be
            // sealed, which happens at the end of its last block.
            let batch = input.creation_block / BATCH_SIZE;
            let sealed_after = batch.checked_add(1).and_then(|b| b.checked_mul(BATCH_SIZE));
            let Some(sealed_after) = sealed_after else {
                return Ok(None);
            };
            if block_number < sealed_after {
                // The batch is not sealed yet — transient, like the ring case.
                return Err(VMError::TxValidation(
                    crate::errors::TxValidationError::UtxoNotYetSpendable,
                ));
            }
            root = fold(
                root,
                input.creation_block % BATCH_SIZE,
                &input.batch_siblings,
            );
            let slot = u256_to_h256(batch_slot_for_block(input.creation_block));
            if vm.read_vault_slot(slot)? != U256::from_big_endian(root.as_bytes()) {
                return Ok(None);
            }
        }

        // Atomic check-and-set of the spent bit, through the durable tier: the
        // read sees bits staged by earlier frames of this same transaction, so a
        // duplicate across frames fails here rather than double-spending.
        let (slot_u256, mask) = spent_bit_location(input.index);
        let slot = u256_to_h256(slot_u256);
        let word = vm.read_vault_slot(slot)?;
        if is_spent(word, input.index) {
            return Ok(None);
        }
        vm.stage_durable_vault_write(slot, word | mask);
        vm.durable_state_gas = vm.durable_state_gas.saturating_add(state_per_spent_bit);

        spent_value = match spent_value.checked_add(input.value) {
            Some(v) => v,
            None => return Ok(None),
        };
    }

    // --- Conservation and payer -------------------------------------------
    let mut signed_out = U256::zero();
    let outputs: Vec<(Address, U256)> = spend
        .utxo_outs
        .iter()
        .chain(spend.account_outs.iter())
        .map(|o| (o.recipient, o.value))
        .collect();
    for (j, (_, value)) in outputs.iter().enumerate() {
        if u64::try_from(j).is_ok_and(|j| j == spend.change_index) {
            continue; // the change entry is signed with zero and excluded
        }
        signed_out = match signed_out.checked_add(*value) {
            Some(v) => v,
            None => return Ok(None),
        };
    }

    let self_funded = spend.is_self_funded();
    if self_funded {
        // The vault fronts the transaction's maximum cost, so the inputs must
        // cover the outputs AND that cost before a payer exists. This is what an
        // opcode running after gas prepayment could not express.
        let needed = match signed_out.checked_add(max_cost) {
            Some(v) => v,
            None => return Ok(None),
        };
        if spent_value < needed {
            return Ok(None);
        }
        let ctx = vm
            .frame_tx_context
            .as_mut()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;
        // A self-funded spend is the only frame in its transaction (static rule),
        // so no payer can have been established before this point.
        if ctx.payer_address.is_some() {
            return Ok(None);
        }
        ctx.payer_address = Some(utxo_vault());
        // Collect the transaction's maximum cost from the vault now, exactly as an
        // APPROVE(APPROVE_PAYMENT) frame would debit a paymaster. The standard
        // payer flow then refunds `charged - owed` at the end, so the vault is
        // debited precisely the actual fee — which is the amount settlement
        // subtracts from the change output. Without this debit the refund would
        // credit the vault money it never paid.
        //
        // Solvency: conservation just proved the inputs cover `max_cost`, and the
        // vault custodies that input value, so the debit cannot underflow.
        let vault = utxo_vault();
        if vm.decrease_account_balance(vault, max_cost).is_err() {
            return Ok(None);
        }
    } else {
        // Sponsored: the sponsor pays, so the frame's own conservation excludes
        // the fee. The post-loop check binds the resolved payer to `spend.payer`.
        if spent_value < signed_out {
            return Ok(None);
        }
    }

    // `source` for the created outputs' logs: a single actor is attributable, a
    // multi-actor spend pools value and is attributed to the vault.
    let source = if let [only_actor] = spend.actors.as_slice() {
        *only_actor
    } else {
        utxo_vault()
    };
    // Silence the unused warning on builds where the sender is not otherwise
    // read; the value is part of the frame's authenticated context.
    let _ = tx_sender;
    let _ = chain_id;

    let settlement = UtxoSettlement {
        frame_index,
        source,
        spent_value,
        signed_out,
        // Static validation bounded `change_index` by the output count, which is
        // itself a `usize`, so this cannot truncate.
        change_index: usize::try_from(spend.change_index).unwrap_or(usize::MAX),
        utxo_outs: spend
            .utxo_outs
            .iter()
            .map(|o| (o.recipient, o.value))
            .collect(),
        account_outs: spend
            .account_outs
            .iter()
            .map(|o| (o.recipient, o.value))
            .collect(),
        self_funded,
        new_account_reserve_each: reserve_per_account_out,
    };

    Ok(Some((utxo_frame_gas, settlement)))
}

/// EIP-8312 state gas for one spent bit: 1/256 of a new slot's state gas,
/// rounded up. Derived from the live EIP-8037 parameter rather than stored as a
/// literal, so a repricing flows through.
fn spent_bit_state_gas(vm: &VM<'_>) -> u64 {
    vm.state_gas_storage_set.div_ceil(256)
}

fn u256_to_h256(value: U256) -> ethrex_common::H256 {
    ethrex_common::H256(value.to_big_endian())
}
