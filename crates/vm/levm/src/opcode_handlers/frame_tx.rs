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
    errors::{ExceptionalHalt, OpcodeResult, VMError},
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
/// `max_cost = max_fee_per_gas * max_gas
///           + len(blob_hashes) * 131072 * blob_base_fee`.
/// This is the single definition of "maximum cost": APPROVE (scopes 0x1/0x3)
/// debits it from the payer, TXPARAM(0x06) reports it, and the
/// mempool paymaster reservation reserves it. The end-of-tx refund returns
/// `max_cost - effective_gas_price * total_gas_used - base-rate blob burn`, so
/// the payer nets the effective-rate cost of the gas actually used plus the
/// EIP-4844 blob burn (intrinsic gas is inside `total_gas_used`, so it stays
/// non-refundable).
pub(crate) fn compute_tx_max_cost(ctx: &crate::vm::FrameTxContext) -> Result<U256, VMError> {
    let gas_cost = ctx
        .tx
        .max_fee_per_gas
        .checked_mul(U256::from(ctx.max_gas))
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
                return Err(VMError::RevertOpcode);
            }
            // EIP-8141: payment approval must not precede the sender's execution
            // approval. Per the spec's APPROVE_PAYMENT rules, revert the frame
            // while sender_approved == false (the sender authorizes execution
            // first; only then may a payer be bound and the max cost collected).
            if !ctx.sender_approved {
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

            approve_payment_effects(vm, sender, frame_target, tx_cost)?;

            // EIP-8141 pins the initial `accessed_addresses` set and adds the payer
            // to it when an APPROVE with payment scope binds it and collects
            // `max_cost`, as for any protocol-touched account. Warm it explicitly
            // rather than relying on the frame-entry access charge to have done it:
            // the protocol touch is the reason it is warm, and a payer bound from
            // the protocol default code never went through a frame's EVM entry.
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
                return Err(VMError::RevertOpcode);
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
                return Err(VMError::RevertOpcode);
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

            approve_payment_effects(vm, sender, frame_target, tx_cost)?;

            // EIP-8141 pins the initial `accessed_addresses` set and adds the payer
            // to it when an APPROVE with payment scope binds it and collects
            // `max_cost`, as for any protocol-touched account. Warm it explicitly
            // rather than relying on the frame-entry access charge to have done it:
            // the protocol touch is the reason it is warm, and a payer bound from
            // the protocol default code never went through a frame's EVM entry.
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

/// The once-per-transaction payment effects, in EIP-8141's order as amended by
/// EIP-8250 §Nonce consumption. An underfunded payer reverts the frame before
/// anything is touched. The state gas the nonce set owes — sender account
/// creation for key 0, one storage set per fresh keyed slot — is then charged
/// from the frame's `limits.state`, and a frame that cannot cover it halts with
/// no effect applied. Only then are the nonces consumed and `max_cost`
/// collected; the caller binds `payer` (and `sender_approved`) right after, so
/// the effects land together or not at all.
fn approve_payment_effects(
    vm: &mut VM<'_>,
    sender: ethrex_common::Address,
    payer: ethrex_common::Address,
    tx_cost: U256,
) -> Result<(), VMError> {
    if vm.db.get_account(payer)?.info.balance < tx_cost {
        return Err(VMError::RevertOpcode);
    }
    let nonce_state_gas = vm.nonce_state_gas(sender)?;
    vm.increase_state_gas(nonce_state_gas)?;
    vm.consume_keyed_nonces(sender)?;
    // Checked above, so an underflow here is an internal fault rather than a
    // frame-level revert.
    vm.decrease_account_balance(payer, tx_cost)?;
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
        // requested scope must be a non-zero subset of a (necessarily non-zero)
        // allowed_scope. EIP-8141: "Ensure that `scope` is one of the caller's
        // allowed scopes in `frame.flags`, otherwise revert." A refused approval
        // reverts its own call frame and nothing more -- halting the whole frame
        // would forfeit its gas and discard work the frame legitimately did after
        // the refusal, which is what the frame's caller is entitled to keep.
        if scope_val == 0 || scope_val > 3 || (scope_val & u64::from(allowed_scope)) != scope_val {
            return Err(VMError::RevertOpcode);
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
/// EIP-8250's legacy account-nonce read, at `0x0D`.
///
/// It sat at `0x0C` until EIP-8141 claimed that index for `state_gas_left`. ethrex moved
/// it to `0x12` ahead of the EIP; upstream then settled the collision differently, shifting
/// all three of EIP-8250's own indices up by one (`e5cf246ff1`, 2026-08-31), so this is
/// `0x0D` and the guess is retired. The spec's numbering wins over ours every time — the
/// same rule that moved `nonce_keys[0]` off `0x0B`, recorded in `docs/eip-8250.md`.
const TXPARAM_LEGACY_SENDER_NONCE: u64 = 0x0D;

/// EIP-8141 TXPARAM `0x0C`: "`state_gas_left` remaining in the currently executing
/// frame". Served from the live VM pool rather than from `FrameTxContext`, which only
/// carries transaction fields.
const TXPARAM_STATE_GAS_LEFT: u64 = 0x0C;

// The TXPARAM index map, pinned. These ids have moved seven times across three EIPs, twice
// in a single day, and a wrong one does not fail loudly: it returns whatever the
// neighbouring EIP assigned — a reference count where a digest was asked for, and no error.
// The published values are asserted here so a renumbering is a compile error rather than a
// silent wrong answer, the same guard the opcode bytes carry in `opcodes.rs`.
//
// EIP-8141 `state_gas_left`; EIP-8250 `94f5a3e3c1`; EIP-8272 (`824cbc0b0e`) claims no
// index. `0x12` is
// ethrex's own resolved-payer read, which yields to any spec id that lands on it.
const _: () = assert!(TXPARAM_STATE_GAS_LEFT == 0x0C);
const _: () = assert!(TXPARAM_LEGACY_SENDER_NONCE == 0x0D);
const _: () = assert!(TXPARAM_NONCE_KEY_COUNT == 0x0E);
const _: () = assert!(TXPARAM_NONCE_KEYS_HASH == 0x0F);
const _: () = assert!(TXPARAM_NONCE_KEY_0 == 0x10);
const _: () = assert!(TXPARAM_RESOLVED_PAYER == 0x12);

/// EIP-8250 `TXPARAM_NONCE_KEY_COUNT`.
const TXPARAM_NONCE_KEY_COUNT: u64 = 0x0E;
/// EIP-8250 `TXPARAM_NONCE_KEYS_HASH`.
const TXPARAM_NONCE_KEYS_HASH: u64 = 0x0F;
/// EIP-8250 `TXPARAM_NONCE_KEY_0`.
const TXPARAM_NONCE_KEY_0: u64 = 0x10;
/// ethrex-only resolved-payer read, knob-gated on `payerTxparamTime`.
const TXPARAM_RESOLVED_PAYER: u64 = 0x12;

/// Gas cost: 2
pub struct OpTxParamHandler;
impl OpcodeHandler for OpTxParamHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [param_id] = *vm.current_call_frame.stack.pop()?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::TXPARAM)?;

        // Block-invariant knob flag (mirrors env.slot_number): gates the
        // resolved-payer index 0x12 so pre-knob blocks preserve its historical
        // exceptional-halt and re-execute identically.
        let payer_txparam_active = vm.env.config.payer_txparam_active;

        let ctx = vm
            .frame_tx_context
            .as_ref()
            .ok_or(ExceptionalHalt::InvalidOpcode)?;

        let param_id = u64::try_from(param_id).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        // 0x0C is the state gas remaining in the executing frame's pool. It lives on
        // the VM, not the transaction context, so it is answered here.
        if param_id == TXPARAM_STATE_GAS_LEFT {
            let remaining = U256::from(vm.state_gas_reservoir);
            vm.current_call_frame.stack.push(remaining)?;
            return Ok(OpcodeResult::Continue);
        }
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
                let (status, ..) = ctx
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
            0x09 => {
                // limits.state -- the frame's declared state-gas budget
                U256::from(frame.state_gas_limit)
            }
            0x0A | 0x0B => {
                // gas_used.execution (0x0A) and gas_used.state (0x0B) of a past
                // frame. Reading the current or a later frame halts: neither has a
                // recorded figure yet.
                if idx >= ctx.current_frame_index {
                    return Err(ExceptionalHalt::InvalidOpcode.into());
                }
                let result = ctx
                    .frame_results
                    .get(idx)
                    .ok_or(ExceptionalHalt::InvalidOpcode)?;
                if param_id == 0x0A {
                    U256::from(result.1)
                } else {
                    U256::from(result.2)
                }
            }
            _ => return Err(ExceptionalHalt::InvalidOpcode.into()),
        };

        vm.current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }
}

/// SIGPARAM (0xB4) -- signature-scoped metadata and data copy (EIP-8141).
/// Metadata (params 0x00-0x03): stack `[param, signatureIndex]` with
/// `signatureIndex` on top; gas 2; returns one word (0x00 effective signer,
/// 0x01 scheme, 0x02 msg, 0x03 len(signature)). Copy (param 0x04): takes
/// `[signatureIndex, param, memOffset, dataOffset, length]` with `signatureIndex`
/// on top (popped first), matching `CALLDATACOPY`'s operand order; CALLDATACOPY
/// gas; copies an ARBITRARY signature's raw bytes into memory (zero-filled past
/// the end) and pushes nothing — any other scheme halts.
pub struct OpSigParamHandler;
impl OpcodeHandler for OpSigParamHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [signature_index, param] = *vm.current_call_frame.stack.pop()?;
        let param = u64::try_from(param).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let signature_index =
            u64::try_from(signature_index).map_err(|_| ExceptionalHalt::InvalidOpcode)?;
        let idx = index_to_usize(signature_index)?;

        // Metadata (0x00-0x03): fixed gas, returns one word.
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
            0x03 => {
                // `len(signature)` is ARBITRARY-only: the raw bytes of a
                // protocol-validated scheme, their length included, are
                // deliberately not introspectable.
                if sig.scheme != ethrex_common::types::FRAME_SIG_SCHEME_ARBITRARY {
                    return Err(ExceptionalHalt::InvalidOpcode.into());
                }
                U256::from(sig.signature.len())
            }
            _ => return Err(ExceptionalHalt::InvalidOpcode.into()),
        };
        vm.current_call_frame.stack.push(result)?;
        Ok(OpcodeResult::Continue)
    }
}

/// SIGDATACOPY (0xB5) -- copy an `ARBITRARY` signature's raw bytes into memory.
///
/// EIP-8141 split this out of `SIGPARAM` so that `SIGPARAM`'s stack requirement is
/// static: it took two operands for metadata and five for the copy form, which no
/// static analysis could resolve. The copy now has its own opcode with a fixed
/// four-operand shape.
///
/// Stack (top first): `memOffset`, `dataOffset`, `length`, `signatureIndex`. No
/// output. Gas is `CALLDATACOPY`'s: the fixed 3, the per-word copy cost, and
/// memory expansion. The raw bytes of protocol-validated schemes stay
/// unreadable -- referencing one is an exceptional halt -- because they may be
/// aggregated in future, while `ARBITRARY` bytes are validated in EVM execution
/// and so may be read.
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
        // Charge memory expansion before the context/scheme guards: the caller pays
        // for the growth it requested even if the opcode then halts.
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
        0x03 => Ok(ctx.tx.max_priority_fee_per_gas),
        0x04 => Ok(ctx.tx.max_fee_per_gas),
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
        // EIP-8250 keyed nonces, at the indices the EIP settled on once EIP-8141 took
        // `0x0C` for `state_gas_left` (upstream `e5cf246ff1`, 2026-08-31). `0x0C` itself is
        // handled by the TXPARAM opcode rather than here: it reads the live frame pool,
        // which this transaction-only view has no access to.
        TXPARAM_LEGACY_SENDER_NONCE => Ok(U256::from(ctx.legacy_sender_nonce)),
        TXPARAM_NONCE_KEY_COUNT => Ok(U256::from(ctx.tx.nonce_keys.len())),
        TXPARAM_NONCE_KEYS_HASH => Ok(U256::from_big_endian(ctx.tx.nonce_keys_hash().as_bytes())),
        // 0x10 = nonce_keys[0], which the spec pins here (ethrex keeps the EIP-8141
        // allocation `0x0B` for len(signatures); see docs/eip-8250.md).
        TXPARAM_NONCE_KEY_0 => ctx
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
        TXPARAM_RESOLVED_PAYER if payer_txparam_active => Ok(ctx
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
        // A reserved mode is rejected by static validation.
        None => Err(ExceptionalHalt::InvalidOpcode.into()),
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

    fn ctx(max_fee: u64, blobs: usize, blob_base_fee: u64, max_gas: u64) -> FrameTxContext {
        let tx = FrameTransaction {
            max_fee_per_gas: U256::from(max_fee),
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
            outstanding_charge_owners: Default::default(),
            sig_hash: H256::zero(),
            tx,
            approve_called_in_current_frame: false,
            max_gas,
            legacy_sender_nonce: 0,
            blob_base_fee: U256::from(blob_base_fee),
        }
    }

    #[test]
    fn max_cost_is_max_fee_times_limit_plus_base_rate_blob_cost() {
        // 10 * 100_000 + 2 * 131072 * 5 = 1_000_000 + 1_310_720
        let c = ctx(10, 2, 5, 100_000);
        assert_eq!(compute_tx_max_cost(&c).unwrap(), U256::from(2_310_720u64));
        // No blobs: just max_fee * max_gas.
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
    fn txparam_0x12_reads_resolved_payer_when_knob_active() {
        let payer = Address::from_low_u64_be(0xABCD);
        let mut c = ctx(10, 0, 0, 21_000);
        c.payer_address = Some(payer);
        assert_eq!(
            load_tx_param(&c, 0x12, true).unwrap(),
            address_to_u256(payer),
            "0x12 must report the resolved payer when the knob is active"
        );
    }

    #[test]
    fn txparam_0x12_reads_zero_before_payer_resolved() {
        // A validation-prefix VERIFY frame runs before payment is approved.
        let c = ctx(10, 0, 0, 21_000);
        assert!(c.payer_address.is_none());
        assert_eq!(
            load_tx_param(&c, 0x12, true).unwrap(),
            U256::zero(),
            "0x12 must read the zero address before the payer is resolved"
        );
    }

    #[test]
    fn txparam_0x12_halts_when_knob_inactive() {
        // History preservation: before the payer_txparam knob, 0x12 keeps its
        // exceptional halt so already-produced blocks re-execute identically —
        // even when a payer is present.
        let mut c = ctx(10, 0, 0, 21_000);
        c.payer_address = Some(Address::from_low_u64_be(0xABCD));
        assert!(matches!(
            load_tx_param(&c, 0x12, false),
            Err(VMError::ExceptionalHalt(ExceptionalHalt::InvalidOpcode))
        ));
    }

    #[test]
    fn txparam_unknown_index_halts_even_when_knob_active() {
        // 0x13: the first index above every assignment in this rule set. The knob only ever
        // opens 0x12, so everything past the table still halts.
        let c = ctx(10, 0, 0, 21_000);
        assert!(matches!(
            load_tx_param(&c, 0x13, true),
            Err(VMError::ExceptionalHalt(ExceptionalHalt::InvalidOpcode))
        ));
    }
}
