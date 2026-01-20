//! Relocation markers for JIT stencils.
//!
//! These extern declarations create relocations in the compiled object file
//! that we find and patch at JIT time. The actual addresses are filled in
//! when copying stencils to the executable buffer.
//!
//! ## Execution Model (Threaded Code)
//!
//! Stencils now RETURN after each opcode instead of chaining. This avoids
//! stack frame accumulation issues on ARM64. The dispatch loop in execute_jit
//! calls each stencil and checks exit_reason after each one.
//!
//! The only relocation marker still used is IMMEDIATE_VALUE for PUSH.

use super::context::U256;

unsafe extern "C" {
    /// Marker for embedded immediate value (PUSH instructions).
    /// This is a static variable, not a function - it will be
    /// patched with the actual push value at JIT time.
    #[link_name = "IMMEDIATE_VALUE"]
    pub static IMMEDIATE_VALUE: U256;
}
