//! Arithmetic opcode stencils.
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

/// ADD opcode stencil
///
/// Pops two values, pushes their sum.
/// Gas cost: 3
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn stencil_add(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // Gas check
    ctx.gas_remaining -= GAS_ADD;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = EXIT_OUT_OF_GAS;
        return;
    }

    // Stack underflow check (need 2 items)
    if ctx.stack_offset > STACK_LIMIT - 2 {
        ctx.exit_reason = EXIT_STACK_UNDERFLOW;
        return;
    }

    // Pop two values
    let a = *ctx.stack_values.add(ctx.stack_offset);
    let b = *ctx.stack_values.add(ctx.stack_offset + 1);

    // Compute sum
    let result = a.wrapping_add(b);

    // Push result (net: pop 2, push 1 = increment offset by 1)
    ctx.stack_offset += 1;
    *ctx.stack_values.add(ctx.stack_offset) = result;

    // Signal continue
    ctx.exit_reason = EXIT_CONTINUE;
}

/// SUB opcode stencil
///
/// Pops two values, pushes a - b.
/// Gas cost: 3
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn stencil_sub(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // Gas check
    ctx.gas_remaining -= GAS_SUB;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = EXIT_OUT_OF_GAS;
        return;
    }

    // Stack underflow check (need 2 items)
    if ctx.stack_offset > STACK_LIMIT - 2 {
        ctx.exit_reason = EXIT_STACK_UNDERFLOW;
        return;
    }

    // Pop two values
    let a = *ctx.stack_values.add(ctx.stack_offset);
    let b = *ctx.stack_values.add(ctx.stack_offset + 1);

    // Compute difference
    let result = a.wrapping_sub(b);

    // Push result
    ctx.stack_offset += 1;
    *ctx.stack_values.add(ctx.stack_offset) = result;

    // Signal continue
    ctx.exit_reason = EXIT_CONTINUE;
}

/// MUL opcode stencil
///
/// Pops two values, pushes their product.
/// Gas cost: 5
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn stencil_mul(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // Gas check
    ctx.gas_remaining -= GAS_MUL;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = EXIT_OUT_OF_GAS;
        return;
    }

    // Stack underflow check (need 2 items)
    if ctx.stack_offset > STACK_LIMIT - 2 {
        ctx.exit_reason = EXIT_STACK_UNDERFLOW;
        return;
    }

    // Pop two values
    let a = *ctx.stack_values.add(ctx.stack_offset);
    let b = *ctx.stack_values.add(ctx.stack_offset + 1);

    // Compute product
    let result = a.wrapping_mul(b);

    // Push result
    ctx.stack_offset += 1;
    *ctx.stack_values.add(ctx.stack_offset) = result;

    // Signal continue
    ctx.exit_reason = EXIT_CONTINUE;
}
