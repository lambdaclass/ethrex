//! Control flow opcode stencils.
//!
//! These functions are compiled to object files at build time.
//! The bytes are extracted and used as templates for copy-and-patch.
//!
//! ## Execution Model
//!
//! Each stencil processes one opcode and returns. Exit_reason indicates:
//! - EXIT_CONTINUE (0): Continue to next opcode
//! - Other values: Exit JIT execution

#![allow(unsafe_op_in_unsafe_fn)]

use super::context::*;

/// STOP opcode stencil
///
/// Halts execution successfully with no return data.
/// Gas cost: 0
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn stencil_stop(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // STOP has no gas cost
    // Set exit reason to stop (not continue)
    ctx.exit_reason = EXIT_STOP;
}

/// JUMPDEST opcode stencil
///
/// Marks a valid jump destination. This is essentially a no-op
/// except for gas consumption.
/// Gas cost: 1
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn stencil_jumpdest(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // Gas check
    ctx.gas_remaining -= GAS_JUMPDEST;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = EXIT_OUT_OF_GAS;
        return;
    }

    // JUMPDEST is just a marker - continue execution
    ctx.exit_reason = EXIT_CONTINUE;
}

/// PC opcode stencil
///
/// Pushes the current program counter value.
/// Note: The PC value must be set by the dispatch loop before calling.
/// Gas cost: 2
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn stencil_pc(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // Gas check
    ctx.gas_remaining -= GAS_PC;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = EXIT_OUT_OF_GAS;
        return;
    }

    // Stack overflow check
    if ctx.stack_offset == 0 {
        ctx.exit_reason = EXIT_STACK_OVERFLOW;
        return;
    }

    // Push PC value
    ctx.stack_offset -= 1;
    *ctx.stack_values.add(ctx.stack_offset) = U256::from_u64(ctx.pc as u64);

    ctx.exit_reason = EXIT_CONTINUE;
}

/// GAS opcode stencil
///
/// Pushes the remaining gas.
/// Gas cost: 2
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn stencil_gas(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // Gas check (deduct first, then push the remaining amount)
    ctx.gas_remaining -= GAS_GAS;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = EXIT_OUT_OF_GAS;
        return;
    }

    // Stack overflow check
    if ctx.stack_offset == 0 {
        ctx.exit_reason = EXIT_STACK_OVERFLOW;
        return;
    }

    // Push remaining gas (after deducting GAS opcode cost)
    ctx.stack_offset -= 1;
    *ctx.stack_values.add(ctx.stack_offset) = U256::from_u64(ctx.gas_remaining as u64);

    ctx.exit_reason = EXIT_CONTINUE;
}
