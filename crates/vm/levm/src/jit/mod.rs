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
            push_value: U256::zero(),
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

    /// Test: JUMPDEST (just charges gas, no-op)
    #[test]
    fn test_jumpdest() {
        // JUMPDEST STOP
        let bytecode = [0x5b, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP");
        assert_eq!(ctx.gas_remaining, 99, "JUMPDEST consumes 1 gas");
    }

    /// Test: LT comparison
    #[test]
    fn test_lt() {
        // LT STOP (2 < 3 = 1)
        let bytecode = [0x10, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        // Stack: 2, 3 (top to bottom) -> LT: 2 < 3 = true = 1
        let stack_top = STACK_LIMIT - 2;
        stack_values[stack_top] = U256::from(2u64);
        stack_values[stack_top + 1] = U256::from(3u64);

        let mut ctx = make_test_ctx(&mut stack_values, stack_top, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP");
        assert_eq!(ctx.gas_remaining, 97, "LT consumes 3 gas");
        assert_eq!(stack_values[STACK_LIMIT - 1], U256::from(1u64), "2 < 3 = 1");
    }

    /// Test: GT comparison
    #[test]
    fn test_gt() {
        // GT STOP (5 > 3 = 1)
        let bytecode = [0x11, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        // Stack: 5, 3 (top to bottom) -> GT: 5 > 3 = true = 1
        let stack_top = STACK_LIMIT - 2;
        stack_values[stack_top] = U256::from(5u64);
        stack_values[stack_top + 1] = U256::from(3u64);

        let mut ctx = make_test_ctx(&mut stack_values, stack_top, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP");
        assert_eq!(ctx.gas_remaining, 97, "GT consumes 3 gas");
        assert_eq!(stack_values[STACK_LIMIT - 1], U256::from(1u64), "5 > 3 = 1");
    }

    /// Test: EQ comparison
    #[test]
    fn test_eq() {
        // EQ STOP (5 == 5 = 1)
        let bytecode = [0x14, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        // Stack: 5, 5 -> EQ: 5 == 5 = true = 1
        let stack_top = STACK_LIMIT - 2;
        stack_values[stack_top] = U256::from(5u64);
        stack_values[stack_top + 1] = U256::from(5u64);

        let mut ctx = make_test_ctx(&mut stack_values, stack_top, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP");
        assert_eq!(ctx.gas_remaining, 97, "EQ consumes 3 gas");
        assert_eq!(stack_values[STACK_LIMIT - 1], U256::from(1u64), "5 == 5 = 1");
    }

    /// Test: ISZERO
    #[test]
    fn test_iszero() {
        // ISZERO STOP (0 == 0 = 1)
        let bytecode = [0x15, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        // Stack: 0 -> ISZERO: 0 == 0 = true = 1
        let stack_top = STACK_LIMIT - 1;
        stack_values[stack_top] = U256::zero();

        let mut ctx = make_test_ctx(&mut stack_values, stack_top, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP");
        assert_eq!(ctx.gas_remaining, 97, "ISZERO consumes 3 gas");
        assert_eq!(stack_values[STACK_LIMIT - 1], U256::from(1u64), "ISZERO(0) = 1");
    }

    /// Test: PC opcode
    #[test]
    fn test_pc() {
        // PC STOP (at PC=0)
        let bytecode = [0x58, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);
        ctx.pc = 0; // PC is 0 at start

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP");
        assert_eq!(ctx.gas_remaining, 98, "PC consumes 2 gas");
        assert_eq!(stack_values[STACK_LIMIT - 1], U256::from(0u64), "PC at position 0");
    }

    /// Test: GAS opcode
    #[test]
    fn test_gas() {
        // GAS STOP
        let bytecode = [0x5a, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP");
        assert_eq!(ctx.gas_remaining, 98, "GAS consumes 2 gas");
        // After GAS opcode, remaining is 98, but the value pushed is the gas AFTER the opcode cost
        assert_eq!(stack_values[STACK_LIMIT - 1], U256::from(98u64), "GAS should push remaining gas");
    }

    /// Test: Simple JUMP
    /// Bytecode: PUSH1 4 JUMP INVALID JUMPDEST STOP
    /// PC:       0    1 2    3       4        5
    #[test]
    fn test_jump() {
        // PUSH1 4, JUMP, 0xFE (invalid), JUMPDEST, STOP
        let bytecode = [0x60, 0x04, 0x56, 0xFE, 0x5b, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP after jump");
        // Gas: PUSH1(3) + JUMP(8) + JUMPDEST(1) = 12
        assert_eq!(ctx.gas_remaining, 88, "Should consume 12 gas");
    }

    /// Test: JUMPI with true condition
    /// Bytecode: PUSH1 1 PUSH1 5 JUMPI INVALID JUMPDEST STOP
    /// PC:       0    1 2    3 4     5       6        7
    #[test]
    fn test_jumpi_true() {
        // PUSH1 1 (condition), PUSH1 6 (dest), JUMPI, 0xFE (invalid), JUMPDEST, STOP
        let bytecode = [0x60, 0x01, 0x60, 0x06, 0x57, 0xFE, 0x5b, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP after jump");
        // Gas: PUSH1(3) + PUSH1(3) + JUMPI(10) + JUMPDEST(1) = 17
        assert_eq!(ctx.gas_remaining, 83, "Should consume 17 gas");
    }

    /// Test: JUMPI with false condition (no jump)
    /// Bytecode: PUSH1 0 PUSH1 6 JUMPI STOP JUMPDEST STOP
    /// PC:       0    1 2    3 4     5    6        7
    #[test]
    fn test_jumpi_false() {
        // PUSH1 0 (condition=false), PUSH1 6 (dest), JUMPI, STOP, JUMPDEST, STOP
        let bytecode = [0x60, 0x00, 0x60, 0x06, 0x57, 0x00, 0x5b, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP (no jump)");
        // Gas: PUSH1(3) + PUSH1(3) + JUMPI(10) + STOP(0) = 16
        assert_eq!(ctx.gas_remaining, 84, "Should consume 16 gas");
    }

    /// Test: Simple loop (count down from 3 to 0)
    /// Pre-populated stack: [3]
    /// Loop: JUMPDEST DUP1 ISZERO PUSH1 <exit> JUMPI PUSH1 1 SWAP1 SUB PUSH1 0 JUMP
    ///       JUMPDEST STOP
    #[test]
    fn test_loop() {
        // A simple countdown loop:
        // 0:  JUMPDEST (loop start)
        // 1:  DUP1 (copy counter)
        // 2:  ISZERO (check if zero)
        // 3:  PUSH1 14 (exit address)
        // 5:  JUMPI (jump to exit if zero)
        // 6:  PUSH1 1
        // 8:  SWAP1
        // 9:  SUB (counter - 1)
        // 10: PUSH1 0 (loop start)
        // 12: JUMP
        // 13: (invalid/unreachable)
        // 14: JUMPDEST (exit)
        // 15: POP (clean up counter)
        // 16: STOP
        let bytecode = [
            0x5b,       // 0: JUMPDEST
            0x80,       // 1: DUP1
            0x15,       // 2: ISZERO
            0x60, 0x0e, // 3: PUSH1 14
            0x57,       // 5: JUMPI
            0x60, 0x01, // 6: PUSH1 1
            0x90,       // 8: SWAP1
            0x03,       // 9: SUB
            0x60, 0x00, // 10: PUSH1 0
            0x56,       // 12: JUMP
            0xFE,       // 13: INVALID (unreachable)
            0x5b,       // 14: JUMPDEST (exit)
            0x50,       // 15: POP
            0x00,       // 16: STOP
        ];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        // Pre-populate stack with counter = 3
        let stack_top = STACK_LIMIT - 1;
        stack_values[stack_top] = U256::from(3u64);

        let mut ctx = make_test_ctx(&mut stack_values, stack_top, 1000, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop, "Should exit with STOP after loop");
        assert_eq!(ctx.stack_offset, STACK_LIMIT, "Stack should be empty after POP");

        // Gas calculation:
        // First iteration (counter=3): JUMPDEST(1) + DUP1(3) + ISZERO(3) + PUSH1(3) + JUMPI(10) + PUSH1(3) + SWAP1(3) + SUB(3) + PUSH1(3) + JUMP(8) = 40
        // Second iteration (counter=2): same = 40
        // Third iteration (counter=1): same = 40
        // Fourth iteration (counter=0): JUMPDEST(1) + DUP1(3) + ISZERO(3) + PUSH1(3) + JUMPI(10) = 20
        // Exit: JUMPDEST(1) + POP(2) = 3
        // Total: 40*3 + 20 + 3 = 143
        assert_eq!(ctx.gas_remaining, 1000 - 143, "Should consume 143 gas for 3-iteration loop");
    }
}
