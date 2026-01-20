//! JIT compiler for EVM bytecode.
//!
//! ## Execution Model: Threaded Code
//!
//! Instead of chaining stencils (which causes stack frame buildup on ARM64),
//! each stencil processes ONE opcode and returns. A Rust dispatch loop in
//! execute_jit calls stencils based on the current PC.
//!
//! This avoids the ARM64 issue where function prologues accumulate stack frames
//! when stencils chain via tail jumps.

#![allow(unsafe_op_in_unsafe_fn)]

use crate::jit::context::{JitContext, JitExitReason};
use crate::jit::executable::{ExecutableBuffer, ExecutableError};
use crate::jit::stencils::{
    RelocKind, Stencil, STENCIL_ADD, STENCIL_MUL, STENCIL_POP, STENCIL_PUSH, STENCIL_STOP,
    STENCIL_SUB,
};

/// Error type for JIT compilation
#[derive(Debug, Clone)]
pub enum JitError {
    /// Executable buffer error
    Executable(ExecutableError),
    /// Unsupported opcode for JIT
    UnsupportedOpcode(u8),
    /// Invalid bytecode
    InvalidBytecode,
    /// JIT compilation disabled
    Disabled,
}

impl std::fmt::Display for JitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Executable(e) => write!(f, "Executable error: {e}"),
            Self::UnsupportedOpcode(op) => write!(f, "Unsupported opcode: 0x{op:02x}"),
            Self::InvalidBytecode => write!(f, "Invalid bytecode"),
            Self::Disabled => write!(f, "JIT compilation disabled"),
        }
    }
}

impl std::error::Error for JitError {}

impl From<ExecutableError> for JitError {
    fn from(e: ExecutableError) -> Self {
        Self::Executable(e)
    }
}

/// A single compiled instruction
#[derive(Clone)]
struct CompiledOp {
    /// Function to call for this opcode
    func: StencilFn,
    /// Size of this instruction in bytecode (1 for most, 2-33 for PUSH)
    size: usize,
}

/// Function signature for stencil functions
type StencilFn = unsafe extern "C" fn(*mut JitContext);

/// JIT-compiled code ready for execution
pub struct JitCode {
    /// Compiled operations indexed by bytecode PC
    ops: Vec<Option<CompiledOp>>,
    /// Executable buffer for PUSH stencils (with patched immediates)
    buffer: ExecutableBuffer,
    /// Maps PC to buffer offset for PUSH instructions
    push_offsets: Vec<Option<usize>>,
}

impl JitCode {
    /// Get the number of bytecode instructions
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

/// JIT compiler for EVM bytecode
pub struct JitCompiler {
    /// Whether JIT compilation is enabled
    enabled: bool,
}

impl Default for JitCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl JitCompiler {
    /// Create a new JIT compiler
    pub fn new() -> Self {
        Self { enabled: true }
    }

    /// Enable or disable JIT compilation
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if JIT compilation is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Compile EVM bytecode to threaded code.
    ///
    /// Creates a dispatch table mapping each bytecode PC to a stencil function.
    /// PUSH instructions get special handling with patched immediate values.
    pub fn compile(&self, bytecode: &[u8]) -> Result<JitCode, JitError> {
        if !self.enabled {
            return Err(JitError::Disabled);
        }

        if bytecode.is_empty() {
            return Err(JitError::InvalidBytecode);
        }

        let mut ops: Vec<Option<CompiledOp>> = vec![None; bytecode.len()];
        let mut push_offsets: Vec<Option<usize>> = vec![None; bytecode.len()];

        // Estimate buffer size for PUSH stencils
        let estimated_pushes = bytecode.iter().filter(|&&b| (0x60..=0x7f).contains(&b)).count();
        let buffer_size = estimated_pushes.saturating_mul(STENCIL_PUSH.bytes.len()).max(4096);
        let mut buffer = ExecutableBuffer::new(buffer_size)?;

        // First pass: compile each instruction
        let mut pc = 0;
        while pc < bytecode.len() {
            #[allow(clippy::indexing_slicing)]
            let opcode = bytecode[pc];

            let (func, size): (StencilFn, usize) = match opcode {
                // STOP
                0x00 => (get_stencil_fn(&STENCIL_STOP), 1),

                // ADD
                0x01 => (get_stencil_fn(&STENCIL_ADD), 1),

                // MUL
                0x02 => (get_stencil_fn(&STENCIL_MUL), 1),

                // SUB
                0x03 => (get_stencil_fn(&STENCIL_SUB), 1),

                // POP
                0x50 => (get_stencil_fn(&STENCIL_POP), 1),

                // PUSH1 - PUSH32
                0x60..=0x7f => {
                    let n = usize::from(opcode.saturating_sub(0x5f)); // 1-32 bytes
                    let value_start = pc.saturating_add(1);
                    let value_end = value_start.saturating_add(n);

                    if value_end > bytecode.len() {
                        return Err(JitError::InvalidBytecode);
                    }

                    // Copy PUSH stencil and patch immediate value
                    let stencil_offset = buffer.len();
                    buffer.copy_stencil(&STENCIL_PUSH, 0)?; // next_pc unused in new model

                    // Find IMMEDIATE_VALUE relocation and patch it
                    for reloc in STENCIL_PUSH.relocations {
                        if reloc.kind == RelocKind::ImmediateValue {
                            #[allow(clippy::indexing_slicing)]
                            let value_bytes = &bytecode[value_start..value_end];
                            let mut padded = [0u8; 32];
                            // EVM uses big-endian, left-pad with zeros
                            let start_idx = 32usize.saturating_sub(n);
                            padded[start_idx..].copy_from_slice(value_bytes);

                            let patch_offset = stencil_offset.saturating_add(reloc.offset);
                            buffer.patch_immediate(patch_offset, &padded)?;
                        }
                    }

                    push_offsets[pc] = Some(stencil_offset);
                    (get_push_fn_placeholder(), n.saturating_add(1))
                }

                // Unsupported opcode
                _ => return Err(JitError::UnsupportedOpcode(opcode)),
            };

            ops[pc] = Some(CompiledOp { func, size });
            pc = pc.saturating_add(size);
        }

        // Make buffer executable for PUSH stencils
        buffer.make_executable()?;

        // Update PUSH function pointers to point into the executable buffer
        for (pc, offset) in push_offsets.iter().enumerate() {
            if let Some(off) = offset {
                if let Some(ref mut op) = ops[pc] {
                    // SAFETY: buffer is executable and contains valid PUSH stencil code
                    if let Some(func) = unsafe { buffer.get_function::<StencilFn>(*off) } {
                        op.func = func;
                    }
                }
            }
        }

        Ok(JitCode {
            ops,
            buffer,
            push_offsets,
        })
    }
}

/// Get function pointer for a stencil.
///
/// For most stencils (not PUSH), we can get the function pointer directly
/// from the linked stencil library.
fn get_stencil_fn(stencil: &'static Stencil) -> StencilFn {
    // The stencil bytes are the compiled function. We need to make them executable
    // and get a function pointer. For static stencils (ADD, SUB, etc.), we use
    // the linked functions directly.
    //
    // This is a placeholder - in practice, we'll call the stencil functions
    // that are linked into the binary.
    match stencil.bytes.as_ptr() as usize {
        _ if std::ptr::eq(stencil, &STENCIL_STOP) => stencil_stop_wrapper,
        _ if std::ptr::eq(stencil, &STENCIL_ADD) => stencil_add_wrapper,
        _ if std::ptr::eq(stencil, &STENCIL_SUB) => stencil_sub_wrapper,
        _ if std::ptr::eq(stencil, &STENCIL_MUL) => stencil_mul_wrapper,
        _ if std::ptr::eq(stencil, &STENCIL_POP) => stencil_pop_wrapper,
        _ => stencil_stop_wrapper, // fallback
    }
}

/// Placeholder for PUSH functions (will be replaced with buffer function pointer)
fn get_push_fn_placeholder() -> StencilFn {
    stencil_stop_wrapper // Will be overwritten
}

// Wrapper functions that implement the stencil logic directly in Rust.
// These are called when we can't use the extracted stencil bytes.

unsafe extern "C" fn stencil_stop_wrapper(ctx: *mut JitContext) {
    (*ctx).exit_reason = JitExitReason::Stop as u32;
}

unsafe extern "C" fn stencil_add_wrapper(ctx: *mut JitContext) {
    use crate::constants::STACK_LIMIT;
    use ethrex_common::U256;

    let ctx = &mut *ctx;

    // Gas check (ADD costs 3)
    ctx.gas_remaining -= 3;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = JitExitReason::OutOfGas as u32;
        return;
    }

    // Stack underflow check
    if ctx.stack_offset > STACK_LIMIT - 2 {
        ctx.exit_reason = JitExitReason::StackUnderflow as u32;
        return;
    }

    // Pop two values
    let a: U256 = *ctx.stack_values.add(ctx.stack_offset);
    let b: U256 = *ctx.stack_values.add(ctx.stack_offset + 1);

    // Compute sum
    let result = a.overflowing_add(b).0;

    // Push result
    ctx.stack_offset += 1;
    *ctx.stack_values.add(ctx.stack_offset) = result;

    ctx.exit_reason = JitExitReason::Continue as u32;
}

unsafe extern "C" fn stencil_sub_wrapper(ctx: *mut JitContext) {
    use crate::constants::STACK_LIMIT;
    use ethrex_common::U256;

    let ctx = &mut *ctx;

    ctx.gas_remaining -= 3;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = JitExitReason::OutOfGas as u32;
        return;
    }

    if ctx.stack_offset > STACK_LIMIT - 2 {
        ctx.exit_reason = JitExitReason::StackUnderflow as u32;
        return;
    }

    let a: U256 = *ctx.stack_values.add(ctx.stack_offset);
    let b: U256 = *ctx.stack_values.add(ctx.stack_offset + 1);
    let result = a.overflowing_sub(b).0;

    ctx.stack_offset += 1;
    *ctx.stack_values.add(ctx.stack_offset) = result;

    ctx.exit_reason = JitExitReason::Continue as u32;
}

unsafe extern "C" fn stencil_mul_wrapper(ctx: *mut JitContext) {
    use crate::constants::STACK_LIMIT;
    use ethrex_common::U256;

    let ctx = &mut *ctx;

    ctx.gas_remaining -= 5;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = JitExitReason::OutOfGas as u32;
        return;
    }

    if ctx.stack_offset > STACK_LIMIT - 2 {
        ctx.exit_reason = JitExitReason::StackUnderflow as u32;
        return;
    }

    let a: U256 = *ctx.stack_values.add(ctx.stack_offset);
    let b: U256 = *ctx.stack_values.add(ctx.stack_offset + 1);
    let result = a.overflowing_mul(b).0;

    ctx.stack_offset += 1;
    *ctx.stack_values.add(ctx.stack_offset) = result;

    ctx.exit_reason = JitExitReason::Continue as u32;
}

unsafe extern "C" fn stencil_pop_wrapper(ctx: *mut JitContext) {
    use crate::constants::STACK_LIMIT;

    let ctx = &mut *ctx;

    ctx.gas_remaining -= 2;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = JitExitReason::OutOfGas as u32;
        return;
    }

    if ctx.stack_offset > STACK_LIMIT - 1 {
        ctx.exit_reason = JitExitReason::StackUnderflow as u32;
        return;
    }

    ctx.stack_offset += 1;
    ctx.exit_reason = JitExitReason::Continue as u32;
}

/// Execute JIT-compiled code using threaded dispatch.
///
/// # Safety
///
/// This function executes compiled code and requires:
/// - `code` must be valid JIT-compiled code
/// - `ctx` must be a valid JitContext with proper pointers
pub unsafe fn execute_jit(code: &JitCode, ctx: &mut JitContext) -> JitExitReason {
    let mut pc = 0;

    loop {
        // Check if we've reached the end of bytecode
        if pc >= code.ops.len() {
            return JitExitReason::Stop;
        }

        // Get the compiled op for this PC
        #[allow(clippy::indexing_slicing)]
        let Some(ref op) = code.ops[pc] else {
            // No instruction at this PC (might be in the middle of a PUSH value)
            pc = pc.saturating_add(1);
            continue;
        };

        // Call the stencil function
        (op.func)(ctx);

        // Check exit reason
        let exit_reason = ctx.exit_reason();
        if exit_reason != JitExitReason::Continue {
            return exit_reason;
        }

        // Advance PC
        pc = pc.saturating_add(op.size);
    }
}
