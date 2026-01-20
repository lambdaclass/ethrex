//! # JIT Compiler for LEVM
//!
//! A baseline JIT compiler using copy-and-patch where stencils are the actual
//! Rust opcode implementations, compiled and extracted at build time.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │  Source (Rust)          Build Time            Runtime       │
//! │                                                             │
//! │  stencil_add() ───────► .o file ───────► bytes[] + relocs  │
//! │  {                      (object crate)                      │
//! │    let a = pop();                            │              │
//! │    let b = pop();                            ▼              │
//! │    push(a + b);                          copy bytes         │
//! │    NEXT();                               patch relocs       │
//! │  }                                       make executable    │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Usage
//!
//! - **Deployment code (initcode)**: Interpreted only
//! - **Runtime code**: JIT-compiled on deployment, stored in cache, executed from cache

pub mod compiler;
pub mod context;
pub mod executable;
pub mod stencils;

pub use compiler::{JitCode, JitCompiler, execute_jit};
pub use context::{JitContext, JitExitReason};
pub use executable::ExecutableBuffer;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::STACK_LIMIT;
    use context::{JitContext, JitExitReason, JmpBuf};
    use ethrex_common::U256;

    /// Helper to create a test JitContext
    fn make_test_ctx(
        stack_values: &mut [U256; STACK_LIMIT],
        stack_offset: usize,
        gas: i64,
        bytecode: &[u8],
    ) -> JitContext {
        JitContext {
            stack_values: stack_values.as_mut_ptr(),
            stack_offset,
            gas_remaining: gas,
            memory_ptr: std::ptr::null_mut(),
            memory_size: 0,
            memory_capacity: 0,
            pc: 0,
            bytecode: bytecode.as_ptr(),
            bytecode_len: bytecode.len(),
            jump_table: std::ptr::null(),
            vm_ptr: std::ptr::null_mut(),
            exit_reason: 0,
            return_offset: 0,
            return_size: 0,
            jmp_buf: Default::default(),
            exit_callback: None,
        }
    }

    /// Test: Just STOP
    /// Verifies basic JIT infrastructure works
    #[test]
    fn test_just_stop() {
        let bytecode = [0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP");
        assert_eq!(ctx.gas_remaining, 100, "STOP consumes no gas");
    }

    /// Test: ADD with pre-populated stack then STOP
    #[test]
    fn test_add_stop() {
        let bytecode = [0x01, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        // Pre-populate stack with two values: 2 and 3
        let stack_top = STACK_LIMIT - 2;
        stack_values[stack_top] = U256::from(2u64);
        stack_values[stack_top + 1] = U256::from(3u64);

        let mut ctx = make_test_ctx(&mut stack_values, stack_top, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP");
        assert_eq!(ctx.gas_remaining, 97, "ADD consumes 3 gas");
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Stack should have 1 item");
        assert_eq!(stack_values[STACK_LIMIT - 1], U256::from(5u64), "2 + 3 = 5");
    }

    /// Test: Multiple arithmetic ops (SUB MUL STOP)
    #[test]
    fn test_sub_mul_stop() {
        let bytecode = [0x03, 0x02, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        // Pre-populate stack: 10, 3, 2 (top to bottom)
        // SUB: 10 - 3 = 7
        // MUL: 7 * 2 = 14
        let stack_top = STACK_LIMIT - 3;
        stack_values[stack_top] = U256::from(10u64);
        stack_values[stack_top + 1] = U256::from(3u64);
        stack_values[stack_top + 2] = U256::from(2u64);

        let mut ctx = make_test_ctx(&mut stack_values, stack_top, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP");
        assert_eq!(ctx.gas_remaining, 92, "SUB(3) + MUL(5) = 8 gas");
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Stack should have 1 item");
        assert_eq!(stack_values[STACK_LIMIT - 1], U256::from(14u64), "(10-3)*2 = 14");
    }

    /// Test: POP operation
    #[test]
    fn test_pop_stop() {
        let bytecode = [0x50, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let stack_top = STACK_LIMIT - 1;
        stack_values[stack_top] = U256::from(42u64);

        let mut ctx = make_test_ctx(&mut stack_values, stack_top, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP");
        assert_eq!(ctx.gas_remaining, 98, "POP consumes 2 gas");
        assert_eq!(ctx.stack_offset, STACK_LIMIT, "Stack should be empty");
    }
}
