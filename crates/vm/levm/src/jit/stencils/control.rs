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
