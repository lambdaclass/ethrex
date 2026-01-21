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
    RelocKind, Stencil, STENCIL_ADD, STENCIL_EQ, STENCIL_GAS, STENCIL_GT, STENCIL_ISZERO,
    STENCIL_JUMPDEST, STENCIL_LT, STENCIL_MUL, STENCIL_PC, STENCIL_POP, STENCIL_PUSH,
    STENCIL_STOP, STENCIL_SUB,
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
    #[allow(dead_code)]
    buffer: ExecutableBuffer,
    /// Maps PC to buffer offset for PUSH instructions
    #[allow(dead_code)]
    push_offsets: Vec<Option<usize>>,
    /// Valid jump destinations (PCs where JUMPDEST exists)
    valid_jumpdests: Vec<bool>,
    /// Push values indexed by bytecode PC (only valid for PUSH instructions)
    push_values: Vec<Option<ethrex_common::U256>>,
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

    /// Check if a PC is a valid jump destination
    pub fn is_valid_jumpdest(&self, pc: usize) -> bool {
        pc < self.valid_jumpdests.len() && self.valid_jumpdests[pc]
    }

    /// Get the push value for a PUSH instruction at the given PC
    pub fn get_push_value(&self, pc: usize) -> Option<ethrex_common::U256> {
        self.push_values.get(pc).copied().flatten()
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
        let push_offsets: Vec<Option<usize>> = vec![None; bytecode.len()];
        let mut valid_jumpdests: Vec<bool> = vec![false; bytecode.len()];
        let mut push_values: Vec<Option<ethrex_common::U256>> = vec![None; bytecode.len()];

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

                // LT
                0x10 => (get_stencil_fn(&STENCIL_LT), 1),

                // GT
                0x11 => (get_stencil_fn(&STENCIL_GT), 1),

                // EQ
                0x14 => (get_stencil_fn(&STENCIL_EQ), 1),

                // ISZERO
                0x15 => (get_stencil_fn(&STENCIL_ISZERO), 1),

                // POP
                0x50 => (get_stencil_fn(&STENCIL_POP), 1),

                // PC
                0x58 => (get_stencil_fn(&STENCIL_PC), 1),

                // GAS
                0x5a => (get_stencil_fn(&STENCIL_GAS), 1),

                // JUMP
                0x56 => (stencil_jump_wrapper, 1),

                // JUMPI
                0x57 => (stencil_jumpi_wrapper, 1),

                // JUMPDEST
                0x5b => {
                    valid_jumpdests[pc] = true;
                    (get_stencil_fn(&STENCIL_JUMPDEST), 1)
                }

                // DUP1 - DUP16
                0x80..=0x8f => {
                    let depth = usize::from(opcode - 0x80 + 1); // 1-16
                    (get_dup_wrapper(depth), 1)
                }

                // SWAP1 - SWAP16
                0x90..=0x9f => {
                    let depth = usize::from(opcode - 0x90 + 1); // 1-16
                    (get_swap_wrapper(depth), 1)
                }

                // INVALID
                0xfe => (stencil_invalid_wrapper, 1),

                // PUSH1 - PUSH32
                0x60..=0x7f => {
                    let n = usize::from(opcode.saturating_sub(0x5f)); // 1-32 bytes
                    let value_start = pc.saturating_add(1);
                    let value_end = value_start.saturating_add(n);

                    if value_end > bytecode.len() {
                        return Err(JitError::InvalidBytecode);
                    }

                    // Extract the push value (big-endian, left-padded with zeros)
                    #[allow(clippy::indexing_slicing)]
                    let value_bytes = &bytecode[value_start..value_end];
                    let mut padded = [0u8; 32];
                    let start_idx = 32usize.saturating_sub(n);
                    padded[start_idx..].copy_from_slice(value_bytes);
                    // Convert from big-endian bytes to U256
                    let value = ethrex_common::U256::from_big_endian(&padded);
                    push_values[pc] = Some(value);

                    (stencil_push_wrapper, n.saturating_add(1))
                }

                // Unsupported opcode
                _ => return Err(JitError::UnsupportedOpcode(opcode)),
            };

            ops[pc] = Some(CompiledOp { func, size });
            pc = pc.saturating_add(size);
        }

        // Make buffer executable for PUSH stencils
        buffer.make_executable()?;

        Ok(JitCode {
            ops,
            buffer,
            push_offsets,
            valid_jumpdests,
            push_values,
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
        _ if std::ptr::eq(stencil, &STENCIL_JUMPDEST) => stencil_jumpdest_wrapper,
        _ if std::ptr::eq(stencil, &STENCIL_PC) => stencil_pc_wrapper,
        _ if std::ptr::eq(stencil, &STENCIL_GAS) => stencil_gas_wrapper,
        _ if std::ptr::eq(stencil, &STENCIL_LT) => stencil_lt_wrapper,
        _ if std::ptr::eq(stencil, &STENCIL_GT) => stencil_gt_wrapper,
        _ if std::ptr::eq(stencil, &STENCIL_EQ) => stencil_eq_wrapper,
        _ if std::ptr::eq(stencil, &STENCIL_ISZERO) => stencil_iszero_wrapper,
        _ => stencil_stop_wrapper, // fallback
    }
}

/// Get a DUP wrapper for the given depth (1-16)
fn get_dup_wrapper(depth: usize) -> StencilFn {
    match depth {
        1 => stencil_dup1_wrapper,
        2 => stencil_dup2_wrapper,
        3 => stencil_dup3_wrapper,
        4 => stencil_dup4_wrapper,
        5 => stencil_dup5_wrapper,
        6 => stencil_dup6_wrapper,
        7 => stencil_dup7_wrapper,
        8 => stencil_dup8_wrapper,
        9 => stencil_dup9_wrapper,
        10 => stencil_dup10_wrapper,
        11 => stencil_dup11_wrapper,
        12 => stencil_dup12_wrapper,
        13 => stencil_dup13_wrapper,
        14 => stencil_dup14_wrapper,
        15 => stencil_dup15_wrapper,
        16 => stencil_dup16_wrapper,
        _ => stencil_stop_wrapper,
    }
}

/// Get a SWAP wrapper for the given depth (1-16)
fn get_swap_wrapper(depth: usize) -> StencilFn {
    match depth {
        1 => stencil_swap1_wrapper,
        2 => stencil_swap2_wrapper,
        3 => stencil_swap3_wrapper,
        4 => stencil_swap4_wrapper,
        5 => stencil_swap5_wrapper,
        6 => stencil_swap6_wrapper,
        7 => stencil_swap7_wrapper,
        8 => stencil_swap8_wrapper,
        9 => stencil_swap9_wrapper,
        10 => stencil_swap10_wrapper,
        11 => stencil_swap11_wrapper,
        12 => stencil_swap12_wrapper,
        13 => stencil_swap13_wrapper,
        14 => stencil_swap14_wrapper,
        15 => stencil_swap15_wrapper,
        16 => stencil_swap16_wrapper,
        _ => stencil_stop_wrapper,
    }
}

// Wrapper functions that implement the stencil logic directly in Rust.
// These are called when we can't use the extracted stencil bytes.

unsafe extern "C" fn stencil_stop_wrapper(ctx: *mut JitContext) {
    (*ctx).exit_reason = JitExitReason::Stop as u32;
}

unsafe extern "C" fn stencil_invalid_wrapper(ctx: *mut JitContext) {
    (*ctx).exit_reason = JitExitReason::InvalidOpcode as u32;
}

unsafe extern "C" fn stencil_push_wrapper(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // Gas check (PUSH costs 3)
    ctx.gas_remaining -= 3;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = JitExitReason::OutOfGas as u32;
        return;
    }

    // Stack overflow check
    if ctx.stack_offset == 0 {
        ctx.exit_reason = JitExitReason::StackOverflow as u32;
        return;
    }

    // Push the value (set by execute_jit before calling this wrapper)
    ctx.stack_offset -= 1;
    *ctx.stack_values.add(ctx.stack_offset) = ctx.push_value;

    ctx.exit_reason = JitExitReason::Continue as u32;
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

unsafe extern "C" fn stencil_jumpdest_wrapper(ctx: *mut JitContext) {
    let ctx = &mut *ctx;

    // Gas check (JUMPDEST costs 1)
    ctx.gas_remaining -= 1;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = JitExitReason::OutOfGas as u32;
        return;
    }

    // JUMPDEST is just a marker - continue
    ctx.exit_reason = JitExitReason::Continue as u32;
}

unsafe extern "C" fn stencil_pc_wrapper(ctx: *mut JitContext) {
    use crate::constants::STACK_LIMIT;
    use ethrex_common::U256;

    let ctx = &mut *ctx;

    // Gas check (PC costs 2)
    ctx.gas_remaining -= 2;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = JitExitReason::OutOfGas as u32;
        return;
    }

    // Stack overflow check
    if ctx.stack_offset == 0 {
        ctx.exit_reason = JitExitReason::StackOverflow as u32;
        return;
    }

    // Push PC value
    ctx.stack_offset -= 1;
    *ctx.stack_values.add(ctx.stack_offset) = U256::from(ctx.pc as u64);

    ctx.exit_reason = JitExitReason::Continue as u32;
}

unsafe extern "C" fn stencil_gas_wrapper(ctx: *mut JitContext) {
    use crate::constants::STACK_LIMIT;
    use ethrex_common::U256;

    let ctx = &mut *ctx;

    // Gas check (GAS costs 2)
    ctx.gas_remaining -= 2;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = JitExitReason::OutOfGas as u32;
        return;
    }

    // Stack overflow check
    if ctx.stack_offset == 0 {
        ctx.exit_reason = JitExitReason::StackOverflow as u32;
        return;
    }

    // Push remaining gas
    ctx.stack_offset -= 1;
    *ctx.stack_values.add(ctx.stack_offset) = U256::from(ctx.gas_remaining as u64);

    ctx.exit_reason = JitExitReason::Continue as u32;
}

unsafe extern "C" fn stencil_lt_wrapper(ctx: *mut JitContext) {
    use crate::constants::STACK_LIMIT;
    use ethrex_common::U256;

    let ctx = &mut *ctx;

    // Gas check (LT costs 3)
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

    // Compute result
    let result = if a < b { U256::from(1u64) } else { U256::zero() };

    // Push result
    ctx.stack_offset += 1;
    *ctx.stack_values.add(ctx.stack_offset) = result;

    ctx.exit_reason = JitExitReason::Continue as u32;
}

unsafe extern "C" fn stencil_gt_wrapper(ctx: *mut JitContext) {
    use crate::constants::STACK_LIMIT;
    use ethrex_common::U256;

    let ctx = &mut *ctx;

    // Gas check (GT costs 3)
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

    // Compute result
    let result = if a > b { U256::from(1u64) } else { U256::zero() };

    // Push result
    ctx.stack_offset += 1;
    *ctx.stack_values.add(ctx.stack_offset) = result;

    ctx.exit_reason = JitExitReason::Continue as u32;
}

unsafe extern "C" fn stencil_eq_wrapper(ctx: *mut JitContext) {
    use crate::constants::STACK_LIMIT;
    use ethrex_common::U256;

    let ctx = &mut *ctx;

    // Gas check (EQ costs 3)
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

    // Compute result
    let result = if a == b { U256::from(1u64) } else { U256::zero() };

    // Push result
    ctx.stack_offset += 1;
    *ctx.stack_values.add(ctx.stack_offset) = result;

    ctx.exit_reason = JitExitReason::Continue as u32;
}

unsafe extern "C" fn stencil_iszero_wrapper(ctx: *mut JitContext) {
    use crate::constants::STACK_LIMIT;
    use ethrex_common::U256;

    let ctx = &mut *ctx;

    // Gas check (ISZERO costs 3)
    ctx.gas_remaining -= 3;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = JitExitReason::OutOfGas as u32;
        return;
    }

    // Stack underflow check
    if ctx.stack_offset > STACK_LIMIT - 1 {
        ctx.exit_reason = JitExitReason::StackUnderflow as u32;
        return;
    }

    // Pop value
    let a: U256 = *ctx.stack_values.add(ctx.stack_offset);

    // Compute result
    let result = if a.is_zero() { U256::from(1u64) } else { U256::zero() };

    // Overwrite top (pop 1, push 1)
    *ctx.stack_values.add(ctx.stack_offset) = result;

    ctx.exit_reason = JitExitReason::Continue as u32;
}

unsafe extern "C" fn stencil_jump_wrapper(ctx: *mut JitContext) {
    use crate::constants::STACK_LIMIT;
    use ethrex_common::U256;

    let ctx = &mut *ctx;

    // Gas check (JUMP costs 8)
    ctx.gas_remaining -= 8;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = JitExitReason::OutOfGas as u32;
        return;
    }

    // Stack underflow check
    if ctx.stack_offset > STACK_LIMIT - 1 {
        ctx.exit_reason = JitExitReason::StackUnderflow as u32;
        return;
    }

    // Pop destination
    let dest: U256 = *ctx.stack_values.add(ctx.stack_offset);
    ctx.stack_offset += 1;

    // Check if destination fits in usize
    if dest > U256::from(usize::MAX) {
        ctx.exit_reason = JitExitReason::InvalidJump as u32;
        return;
    }

    // Set pc to destination (validation happens in dispatch loop)
    ctx.pc = dest.as_usize();

    // Signal jump taken
    ctx.exit_reason = JitExitReason::Jump as u32;
}

unsafe extern "C" fn stencil_jumpi_wrapper(ctx: *mut JitContext) {
    use crate::constants::STACK_LIMIT;
    use ethrex_common::U256;

    let ctx = &mut *ctx;

    // Gas check (JUMPI costs 10)
    ctx.gas_remaining -= 10;
    if ctx.gas_remaining < 0 {
        ctx.exit_reason = JitExitReason::OutOfGas as u32;
        return;
    }

    // Stack underflow check (need 2 items)
    if ctx.stack_offset > STACK_LIMIT - 2 {
        ctx.exit_reason = JitExitReason::StackUnderflow as u32;
        return;
    }

    // Pop destination and condition
    let dest: U256 = *ctx.stack_values.add(ctx.stack_offset);
    let cond: U256 = *ctx.stack_values.add(ctx.stack_offset + 1);
    ctx.stack_offset += 2;

    // If condition is zero, don't jump (continue to next instruction)
    if cond.is_zero() {
        ctx.exit_reason = JitExitReason::Continue as u32;
        return;
    }

    // Check if destination fits in usize
    if dest > U256::from(usize::MAX) {
        ctx.exit_reason = JitExitReason::InvalidJump as u32;
        return;
    }

    // Set pc to destination (validation happens in dispatch loop)
    ctx.pc = dest.as_usize();

    // Signal jump taken
    ctx.exit_reason = JitExitReason::Jump as u32;
}

// DUP wrappers - DUP1 through DUP16
macro_rules! define_dup_wrapper {
    ($name:ident, $depth:expr) => {
        unsafe extern "C" fn $name(ctx: *mut JitContext) {
            use crate::constants::STACK_LIMIT;
            use ethrex_common::U256;

            let ctx = &mut *ctx;

            // Gas check (DUP costs 3)
            ctx.gas_remaining -= 3;
            if ctx.gas_remaining < 0 {
                ctx.exit_reason = JitExitReason::OutOfGas as u32;
                return;
            }

            // Stack underflow check (need depth items)
            if ctx.stack_offset > STACK_LIMIT - $depth {
                ctx.exit_reason = JitExitReason::StackUnderflow as u32;
                return;
            }

            // Stack overflow check
            if ctx.stack_offset == 0 {
                ctx.exit_reason = JitExitReason::StackOverflow as u32;
                return;
            }

            // Get the value at depth - 1 from top (0-indexed)
            let value: U256 = *ctx.stack_values.add(ctx.stack_offset + $depth - 1);

            // Push it
            ctx.stack_offset -= 1;
            *ctx.stack_values.add(ctx.stack_offset) = value;

            ctx.exit_reason = JitExitReason::Continue as u32;
        }
    };
}

define_dup_wrapper!(stencil_dup1_wrapper, 1);
define_dup_wrapper!(stencil_dup2_wrapper, 2);
define_dup_wrapper!(stencil_dup3_wrapper, 3);
define_dup_wrapper!(stencil_dup4_wrapper, 4);
define_dup_wrapper!(stencil_dup5_wrapper, 5);
define_dup_wrapper!(stencil_dup6_wrapper, 6);
define_dup_wrapper!(stencil_dup7_wrapper, 7);
define_dup_wrapper!(stencil_dup8_wrapper, 8);
define_dup_wrapper!(stencil_dup9_wrapper, 9);
define_dup_wrapper!(stencil_dup10_wrapper, 10);
define_dup_wrapper!(stencil_dup11_wrapper, 11);
define_dup_wrapper!(stencil_dup12_wrapper, 12);
define_dup_wrapper!(stencil_dup13_wrapper, 13);
define_dup_wrapper!(stencil_dup14_wrapper, 14);
define_dup_wrapper!(stencil_dup15_wrapper, 15);
define_dup_wrapper!(stencil_dup16_wrapper, 16);

// SWAP wrappers - SWAP1 through SWAP16
macro_rules! define_swap_wrapper {
    ($name:ident, $depth:expr) => {
        unsafe extern "C" fn $name(ctx: *mut JitContext) {
            use crate::constants::STACK_LIMIT;
            use ethrex_common::U256;

            let ctx = &mut *ctx;

            // Gas check (SWAP costs 3)
            ctx.gas_remaining -= 3;
            if ctx.gas_remaining < 0 {
                ctx.exit_reason = JitExitReason::OutOfGas as u32;
                return;
            }

            // Stack underflow check (need depth + 1 items)
            if ctx.stack_offset > STACK_LIMIT - ($depth + 1) {
                ctx.exit_reason = JitExitReason::StackUnderflow as u32;
                return;
            }

            // Swap top with element at depth
            let top_idx = ctx.stack_offset;
            let swap_idx = ctx.stack_offset + $depth;

            let top: U256 = *ctx.stack_values.add(top_idx);
            let other: U256 = *ctx.stack_values.add(swap_idx);

            *ctx.stack_values.add(top_idx) = other;
            *ctx.stack_values.add(swap_idx) = top;

            ctx.exit_reason = JitExitReason::Continue as u32;
        }
    };
}

define_swap_wrapper!(stencil_swap1_wrapper, 1);
define_swap_wrapper!(stencil_swap2_wrapper, 2);
define_swap_wrapper!(stencil_swap3_wrapper, 3);
define_swap_wrapper!(stencil_swap4_wrapper, 4);
define_swap_wrapper!(stencil_swap5_wrapper, 5);
define_swap_wrapper!(stencil_swap6_wrapper, 6);
define_swap_wrapper!(stencil_swap7_wrapper, 7);
define_swap_wrapper!(stencil_swap8_wrapper, 8);
define_swap_wrapper!(stencil_swap9_wrapper, 9);
define_swap_wrapper!(stencil_swap10_wrapper, 10);
define_swap_wrapper!(stencil_swap11_wrapper, 11);
define_swap_wrapper!(stencil_swap12_wrapper, 12);
define_swap_wrapper!(stencil_swap13_wrapper, 13);
define_swap_wrapper!(stencil_swap14_wrapper, 14);
define_swap_wrapper!(stencil_swap15_wrapper, 15);
define_swap_wrapper!(stencil_swap16_wrapper, 16);

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

        // Set ctx.pc before calling stencil (for PC opcode and JUMP validation)
        ctx.pc = pc;

        // Set push_value if this is a PUSH instruction
        if let Some(value) = code.get_push_value(pc) {
            ctx.push_value = value;
        }

        // Call the stencil function
        (op.func)(ctx);

        // Check exit reason
        let exit_reason = ctx.exit_reason();
        match exit_reason {
            JitExitReason::Continue => {
                // Normal execution - advance PC
                pc = pc.saturating_add(op.size);
            }
            JitExitReason::Jump => {
                // JUMP/JUMPI updated ctx.pc - validate and use it
                let dest = ctx.pc;
                if !code.is_valid_jumpdest(dest) {
                    return JitExitReason::InvalidJump;
                }
                pc = dest;
            }
            _ => return exit_reason,
        }
    }
}
