//! Stack opcode stencils.
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
use super::markers::*;

/// POP opcode stencil
///
/// Pops one value from the stack (discards it).
/// Gas cost: 2
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn stencil_pop(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // Gas check
    ctx.gas_remaining -= GAS_POP;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = EXIT_OUT_OF_GAS;
        return;
    }

    // Stack underflow check (need 1 item)
    if ctx.stack_offset > STACK_LIMIT - 1 {
        ctx.exit_reason = EXIT_STACK_UNDERFLOW;
        return;
    }

    // Pop (just increment offset, value is implicitly discarded)
    ctx.stack_offset += 1;

    // Signal continue
    ctx.exit_reason = EXIT_CONTINUE;
}

/// PUSH opcode stencil
///
/// Pushes an immediate value onto the stack.
/// The value is patched at JIT time via IMMEDIATE_VALUE relocation.
/// Gas cost: 3
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn stencil_push(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // Gas check
    ctx.gas_remaining -= GAS_PUSH;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = EXIT_OUT_OF_GAS;
        return;
    }

    // Stack overflow check
    if ctx.stack_offset == 0 {
        ctx.exit_reason = EXIT_STACK_OVERFLOW;
        return;
    }

    // Push the immediate value (patched at JIT time)
    ctx.stack_offset -= 1;
    *ctx.stack_values.add(ctx.stack_offset) = IMMEDIATE_VALUE;

    // Signal continue
    ctx.exit_reason = EXIT_CONTINUE;
}
