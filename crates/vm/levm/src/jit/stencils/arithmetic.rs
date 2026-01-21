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

/// LT opcode stencil
///
/// Pops two values, pushes 1 if a < b, else 0.
/// Gas cost: 3
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn stencil_lt(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // Gas check
    ctx.gas_remaining -= GAS_LT;
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

    // Compute result: 1 if a < b, else 0
    let result = if a.lt(b) { U256::ONE } else { U256::ZERO };

    // Push result
    ctx.stack_offset += 1;
    *ctx.stack_values.add(ctx.stack_offset) = result;

    ctx.exit_reason = EXIT_CONTINUE;
}

/// GT opcode stencil
///
/// Pops two values, pushes 1 if a > b, else 0.
/// Gas cost: 3
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn stencil_gt(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // Gas check
    ctx.gas_remaining -= GAS_GT;
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

    // Compute result: 1 if a > b, else 0
    let result = if a.gt(b) { U256::ONE } else { U256::ZERO };

    // Push result
    ctx.stack_offset += 1;
    *ctx.stack_values.add(ctx.stack_offset) = result;

    ctx.exit_reason = EXIT_CONTINUE;
}

/// EQ opcode stencil
///
/// Pops two values, pushes 1 if a == b, else 0.
/// Gas cost: 3
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn stencil_eq(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // Gas check
    ctx.gas_remaining -= GAS_EQ;
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

    // Compute result: 1 if a == b, else 0
    let result = if a.eq(b) { U256::ONE } else { U256::ZERO };

    // Push result
    ctx.stack_offset += 1;
    *ctx.stack_values.add(ctx.stack_offset) = result;

    ctx.exit_reason = EXIT_CONTINUE;
}

/// ISZERO opcode stencil
///
/// Pops one value, pushes 1 if value == 0, else 0.
/// Gas cost: 3
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn stencil_iszero(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // Gas check
    ctx.gas_remaining -= GAS_ISZERO;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = EXIT_OUT_OF_GAS;
        return;
    }

    // Stack underflow check (need 1 item)
    if ctx.stack_offset > STACK_LIMIT - 1 {
        ctx.exit_reason = EXIT_STACK_UNDERFLOW;
        return;
    }

    // Pop value
    let a = *ctx.stack_values.add(ctx.stack_offset);

    // Compute result: 1 if a == 0, else 0
    let result = if a.is_zero() { U256::ONE } else { U256::ZERO };

    // Overwrite top of stack (same position since we pop 1, push 1)
    *ctx.stack_values.add(ctx.stack_offset) = result;

    ctx.exit_reason = EXIT_CONTINUE;
}
