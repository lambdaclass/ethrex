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
        make_test_ctx_with_env(stack_values, stack_offset, gas, bytecode, &[])
    }

    /// Helper to create a test JitContext with calldata
    fn make_test_ctx_with_env(
        stack_values: &mut [U256; STACK_LIMIT],
        stack_offset: usize,
        gas: i64,
        bytecode: &[u8],
        calldata: &[u8],
    ) -> JitContext {
        // Default test addresses and values
        static TEST_CALLDATA: &[u8] = &[];
        let calldata_ptr = if calldata.is_empty() {
            TEST_CALLDATA.as_ptr()
        } else {
            calldata.as_ptr()
        };

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
            // Environment data with test values
            address: [0x11; 20],   // Test contract address
            caller: [0x22; 20],    // Test caller address
            callvalue: U256::from(1000u64),  // 1000 wei
            calldata_ptr,
            calldata_len: calldata.len(),
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

    /// Test: AND opcode
    #[test]
    fn test_and() {
        // PUSH1 0xFF, PUSH1 0x0F, AND, STOP -> should get 0x0F
        let bytecode = [0x60, 0xff, 0x60, 0x0f, 0x16, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(0x0fu64), "0xFF AND 0x0F = 0x0F");
    }

    /// Test: OR opcode
    #[test]
    fn test_or() {
        // PUSH1 0xF0, PUSH1 0x0F, OR, STOP -> should get 0xFF
        let bytecode = [0x60, 0xf0, 0x60, 0x0f, 0x17, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(0xffu64), "0xF0 OR 0x0F = 0xFF");
    }

    /// Test: XOR opcode
    #[test]
    fn test_xor() {
        // PUSH1 0xFF, PUSH1 0xF0, XOR, STOP -> should get 0x0F
        let bytecode = [0x60, 0xff, 0x60, 0xf0, 0x18, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(0x0fu64), "0xFF XOR 0xF0 = 0x0F");
    }

    /// Test: NOT opcode
    #[test]
    fn test_not() {
        // PUSH1 0, NOT, STOP -> should get MAX (all 1s)
        let bytecode = [0x60, 0x00, 0x19, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::MAX, "NOT 0 = MAX");
    }

    /// Test: BYTE opcode
    #[test]
    fn test_byte() {
        // PUSH1 0xAB, PUSH1 31, BYTE, STOP -> get byte 31 (LSB) = 0xAB
        let bytecode = [0x60, 0xab, 0x60, 0x1f, 0x1a, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(0xabu64), "BYTE(31, 0xAB) = 0xAB");
    }

    /// Test: SHL opcode
    #[test]
    fn test_shl() {
        // PUSH1 1, PUSH1 4, SHL, STOP -> 1 << 4 = 16
        let bytecode = [0x60, 0x01, 0x60, 0x04, 0x1b, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(16u64), "1 << 4 = 16");
    }

    /// Test: SHR opcode
    #[test]
    fn test_shr() {
        // PUSH1 16, PUSH1 4, SHR, STOP -> 16 >> 4 = 1
        let bytecode = [0x60, 0x10, 0x60, 0x04, 0x1c, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(1u64), "16 >> 4 = 1");
    }

    /// Test: SAR opcode (arithmetic shift right)
    #[test]
    fn test_sar() {
        // PUSH32 MAX (all 1s), PUSH1 4, SAR, STOP -> should still be MAX (sign extension)
        // Bytecode: PUSH32 <32 bytes of 0xFF>, PUSH1 4, SAR, STOP
        let mut bytecode = vec![0x7f]; // PUSH32
        bytecode.extend_from_slice(&[0xff; 32]); // 32 bytes of 0xFF
        bytecode.push(0x60); // PUSH1
        bytecode.push(0x04); // 4
        bytecode.push(0x1d); // SAR
        bytecode.push(0x00); // STOP

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        // SAR on MAX (negative in 2's complement) should give MAX (sign extension fills with 1s)
        assert_eq!(stack_values[ctx.stack_offset], U256::MAX, "SAR(4, MAX) = MAX");
    }

    /// Test: MSIZE opcode
    #[test]
    fn test_msize() {
        // MSIZE, STOP -> should push 0 (no memory used yet)
        let bytecode = [0x59, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);
        // Memory size starts at 0
        ctx.memory_size = 0;

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::zero(), "MSIZE should be 0");
    }

    /// Test: DIV opcode (unsigned division)
    #[test]
    fn test_div() {
        // PUSH1 3, PUSH1 10, DIV, STOP -> 10 / 3 = 3
        let bytecode = [0x60, 0x03, 0x60, 0x0a, 0x04, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(3u64), "10 / 3 = 3");
    }

    /// Test: DIV by zero returns zero
    #[test]
    fn test_div_by_zero() {
        // PUSH1 0, PUSH1 10, DIV, STOP -> 10 / 0 = 0 (EVM semantics)
        let bytecode = [0x60, 0x00, 0x60, 0x0a, 0x04, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::zero(), "10 / 0 = 0");
    }

    /// Test: SDIV opcode (signed division)
    #[test]
    fn test_sdiv() {
        // Test: -10 / 3 = -3 (truncate toward zero)
        // -10 in 256-bit two's complement is MAX - 9
        // PUSH1 3, PUSH32 <-10>, SDIV, STOP -> stack: top=-10, second=3 -> -10 / 3 = -3
        let neg_10 = U256::MAX - U256::from(9u64); // -10 in two's complement
        let mut bytecode = vec![0x60, 0x03]; // PUSH1 3
        bytecode.push(0x7f); // PUSH32
        bytecode.extend_from_slice(&neg_10.to_big_endian());
        bytecode.push(0x05); // SDIV
        bytecode.push(0x00); // STOP

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 200, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        // -10 / 3 = -3 (truncate toward zero)
        let neg_3 = U256::MAX - U256::from(2u64); // -3 in two's complement
        assert_eq!(stack_values[ctx.stack_offset], neg_3, "-10 / 3 = -3");
    }

    /// Test: MOD opcode (unsigned modulo)
    #[test]
    fn test_mod() {
        // PUSH1 3, PUSH1 10, MOD, STOP -> 10 % 3 = 1
        let bytecode = [0x60, 0x03, 0x60, 0x0a, 0x06, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(1u64), "10 % 3 = 1");
    }

    /// Test: MOD by zero returns zero
    #[test]
    fn test_mod_by_zero() {
        // PUSH1 0, PUSH1 10, MOD, STOP -> 10 % 0 = 0 (EVM semantics)
        let bytecode = [0x60, 0x00, 0x60, 0x0a, 0x06, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::zero(), "10 % 0 = 0");
    }

    /// Test: SMOD opcode (signed modulo)
    #[test]
    fn test_smod() {
        // Test: -10 % 3 = -1 (sign of dividend)
        // -10 in 256-bit two's complement is MAX - 9
        // PUSH1 3, PUSH32 <-10>, SMOD, STOP -> stack: top=-10, second=3 -> -10 % 3 = -1
        let neg_10 = U256::MAX - U256::from(9u64); // -10 in two's complement
        let mut bytecode = vec![0x60, 0x03]; // PUSH1 3
        bytecode.push(0x7f); // PUSH32
        bytecode.extend_from_slice(&neg_10.to_big_endian());
        bytecode.push(0x07); // SMOD
        bytecode.push(0x00); // STOP

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 200, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        // -10 % 3 = -1 (sign follows dividend)
        let neg_1 = U256::MAX; // -1 in two's complement
        assert_eq!(stack_values[ctx.stack_offset], neg_1, "-10 % 3 = -1");
    }

    /// Test: ADDMOD opcode
    #[test]
    fn test_addmod() {
        // PUSH1 8, PUSH1 10, PUSH1 10, ADDMOD, STOP -> (10 + 10) % 8 = 4
        let bytecode = [0x60, 0x08, 0x60, 0x0a, 0x60, 0x0a, 0x08, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(4u64), "(10 + 10) % 8 = 4");
    }

    /// Test: ADDMOD with modulus 0 returns 0
    #[test]
    fn test_addmod_mod_zero() {
        // PUSH1 0, PUSH1 10, PUSH1 10, ADDMOD, STOP -> (10 + 10) % 0 = 0
        let bytecode = [0x60, 0x00, 0x60, 0x0a, 0x60, 0x0a, 0x08, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::zero(), "(10 + 10) % 0 = 0");
    }

    /// Test: MULMOD opcode
    #[test]
    fn test_mulmod() {
        // PUSH1 8, PUSH1 10, PUSH1 10, MULMOD, STOP -> (10 * 10) % 8 = 4
        let bytecode = [0x60, 0x08, 0x60, 0x0a, 0x60, 0x0a, 0x09, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(4u64), "(10 * 10) % 8 = 4");
    }

    /// Test: EXP opcode (exponentiation)
    #[test]
    fn test_exp() {
        // PUSH1 3, PUSH1 2, EXP, STOP -> 2^3 = 8
        let bytecode = [0x60, 0x03, 0x60, 0x02, 0x0a, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 200, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(8u64), "2^3 = 8");
    }

    /// Test: EXP with larger exponent
    #[test]
    fn test_exp_larger() {
        // PUSH1 10, PUSH1 2, EXP, STOP -> 2^10 = 1024
        let bytecode = [0x60, 0x0a, 0x60, 0x02, 0x0a, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 200, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(1024u64), "2^10 = 1024");
    }

    /// Test: SIGNEXTEND opcode
    #[test]
    fn test_signextend() {
        // Sign extend a negative byte value
        // PUSH1 0xFF (255, or -1 as signed byte), PUSH1 0, SIGNEXTEND, STOP
        // Result should be 0xFFFF...FF (all 1s, which is -1 in 256-bit)
        let bytecode = [0x60, 0xff, 0x60, 0x00, 0x0b, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::MAX, "SIGNEXTEND(0, 0xFF) = -1");
    }

    /// Test: SIGNEXTEND with positive value
    #[test]
    fn test_signextend_positive() {
        // Sign extend a positive byte value
        // PUSH1 0x7F (127, positive byte), PUSH1 0, SIGNEXTEND, STOP
        // Result should still be 0x7F (no sign extension needed)
        let bytecode = [0x60, 0x7f, 0x60, 0x00, 0x0b, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(0x7fu64), "SIGNEXTEND(0, 0x7F) = 0x7F");
    }

    /// Test: ADDRESS opcode
    #[test]
    fn test_address() {
        // ADDRESS, STOP -> push current contract address
        let bytecode = [0x30, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        // Test address is [0x11; 20]
        let expected = U256::from_big_endian(&[0u8; 12].into_iter().chain([0x11u8; 20]).collect::<Vec<_>>());
        assert_eq!(stack_values[ctx.stack_offset], expected, "ADDRESS should return test address");
    }

    /// Test: CALLER opcode
    #[test]
    fn test_caller() {
        // CALLER, STOP -> push msg sender address
        let bytecode = [0x33, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        // Test caller is [0x22; 20]
        let expected = U256::from_big_endian(&[0u8; 12].into_iter().chain([0x22u8; 20]).collect::<Vec<_>>());
        assert_eq!(stack_values[ctx.stack_offset], expected, "CALLER should return test caller");
    }

    /// Test: CALLVALUE opcode
    #[test]
    fn test_callvalue() {
        // CALLVALUE, STOP -> push msg value
        let bytecode = [0x34, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        // Test callvalue is 1000
        assert_eq!(stack_values[ctx.stack_offset], U256::from(1000u64), "CALLVALUE should return 1000");
    }

    /// Test: CALLDATASIZE opcode
    #[test]
    fn test_calldatasize() {
        // CALLDATASIZE, STOP -> push calldata length
        let bytecode = [0x36, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        // Create context with calldata
        let calldata = [0x11, 0x22, 0x33, 0x44];
        let mut ctx = make_test_ctx_with_env(&mut stack_values, STACK_LIMIT, 100, &bytecode, &calldata);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(4u64), "CALLDATASIZE should return 4");
    }

    /// Test: CODESIZE opcode
    #[test]
    fn test_codesize() {
        // CODESIZE, STOP -> push bytecode length
        let bytecode = [0x38, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(2u64), "CODESIZE should return 2");
    }

    /// Test: CALLDATALOAD opcode
    #[test]
    fn test_calldataload() {
        // PUSH1 0, CALLDATALOAD, STOP -> load first 32 bytes of calldata
        let bytecode = [0x60, 0x00, 0x35, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        // Create 32 bytes of calldata
        let mut calldata = [0u8; 32];
        calldata[0] = 0xAB;
        calldata[1] = 0xCD;
        let mut ctx = make_test_ctx_with_env(&mut stack_values, STACK_LIMIT, 100, &bytecode, &calldata);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        let expected = U256::from_big_endian(&calldata);
        assert_eq!(stack_values[ctx.stack_offset], expected, "CALLDATALOAD should load first 32 bytes");
    }

    /// Test: CALLDATALOAD with out-of-bounds offset returns zeros
    #[test]
    fn test_calldataload_oob() {
        // PUSH1 100, CALLDATALOAD, STOP -> load from offset 100 (past end of calldata)
        let bytecode = [0x60, 0x64, 0x35, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        // Only 4 bytes of calldata
        let calldata = [0x11, 0x22, 0x33, 0x44];
        let mut ctx = make_test_ctx_with_env(&mut stack_values, STACK_LIMIT, 100, &bytecode, &calldata);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        assert_eq!(stack_values[ctx.stack_offset], U256::zero(), "CALLDATALOAD OOB should return 0");
    }

    /// Test: RETURN opcode
    #[test]
    fn test_return() {
        // PUSH1 32 (size), PUSH1 0 (offset), RETURN -> return 32 bytes from offset 0
        let bytecode = [0x60, 0x20, 0x60, 0x00, 0xf3];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Return);
        assert_eq!(ctx.return_offset, 0, "Return offset should be 0");
        assert_eq!(ctx.return_size, 32, "Return size should be 32");
        assert_eq!(ctx.stack_offset, STACK_LIMIT, "Stack should be empty after RETURN pops args");
    }

    /// Test: RETURN with zero size
    #[test]
    fn test_return_zero_size() {
        // PUSH1 0 (size), PUSH1 0 (offset), RETURN -> return nothing
        let bytecode = [0x60, 0x00, 0x60, 0x00, 0xf3];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Return);
        assert_eq!(ctx.return_offset, 0, "Return offset should be 0");
        assert_eq!(ctx.return_size, 0, "Return size should be 0");
    }

    /// Test: REVERT opcode
    #[test]
    fn test_revert() {
        // PUSH1 4 (size), PUSH1 0 (offset), REVERT -> revert with 4 bytes from offset 0
        let bytecode = [0x60, 0x04, 0x60, 0x00, 0xfd];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut ctx = make_test_ctx(&mut stack_values, STACK_LIMIT, 100, &bytecode);

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Revert);
        assert_eq!(ctx.return_offset, 0, "Revert offset should be 0");
        assert_eq!(ctx.return_size, 4, "Revert size should be 4");
        assert_eq!(ctx.stack_offset, STACK_LIMIT, "Stack should be empty after REVERT pops args");
    }

    /// Test: MSTORE and MLOAD round-trip
    #[test]
    fn test_mstore_mload() {
        // PUSH1 0x42, PUSH1 0, MSTORE, PUSH1 0, MLOAD, STOP
        // Store 0x42 at offset 0, then load it back
        let bytecode = [0x60, 0x42, 0x60, 0x00, 0x52, 0x60, 0x00, 0x51, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        // Allocate memory for the test
        let mut memory = vec![0u8; 64];

        let mut ctx = JitContext {
            stack_values: stack_values.as_mut_ptr(),
            stack_offset: STACK_LIMIT,
            gas_remaining: 1000,
            memory_ptr: memory.as_mut_ptr(),
            memory_size: 0,
            memory_capacity: memory.len(),
            pc: 0,
            bytecode: bytecode.as_ptr(),
            bytecode_len: bytecode.len(),
            jump_table: std::ptr::null(),
            vm_ptr: std::ptr::null_mut(),
            exit_reason: 0,
            return_offset: 0,
            return_size: 0,
            push_value: U256::zero(),
            address: [0x11; 20],
            caller: [0x22; 20],
            callvalue: U256::from(1000u64),
            calldata_ptr: std::ptr::null(),
            calldata_len: 0,
            jmp_buf: Default::default(),
            exit_callback: None,
        };

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        // The value 0x42 is stored as a 32-byte big-endian word at offset 0
        // When loaded back, it should be 0x42 (padded to 256 bits)
        assert_eq!(stack_values[ctx.stack_offset], U256::from(0x42u64), "MLOAD should return stored value");
        assert_eq!(ctx.memory_size, 32, "Memory size should be 32 after MSTORE");
    }

    /// Test: MSTORE8 stores a single byte
    #[test]
    fn test_mstore8() {
        // PUSH1 0xAB, PUSH1 0, MSTORE8, PUSH1 0, MLOAD, STOP
        // Store byte 0xAB at offset 0, then load 32 bytes starting at 0
        let bytecode = [0x60, 0xAB, 0x60, 0x00, 0x53, 0x60, 0x00, 0x51, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut memory = vec![0u8; 64];

        let mut ctx = JitContext {
            stack_values: stack_values.as_mut_ptr(),
            stack_offset: STACK_LIMIT,
            gas_remaining: 1000,
            memory_ptr: memory.as_mut_ptr(),
            memory_size: 0,
            memory_capacity: memory.len(),
            pc: 0,
            bytecode: bytecode.as_ptr(),
            bytecode_len: bytecode.len(),
            jump_table: std::ptr::null(),
            vm_ptr: std::ptr::null_mut(),
            exit_reason: 0,
            return_offset: 0,
            return_size: 0,
            push_value: U256::zero(),
            address: [0x11; 20],
            caller: [0x22; 20],
            callvalue: U256::from(1000u64),
            calldata_ptr: std::ptr::null(),
            calldata_len: 0,
            jmp_buf: Default::default(),
            exit_callback: None,
        };

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack");
        // 0xAB stored at byte 0 means the 32-byte word starting at 0 is 0xAB000...000
        let expected = U256::from(0xABu64) << 248; // 0xAB in the most significant byte
        assert_eq!(stack_values[ctx.stack_offset], expected, "MSTORE8 should store byte at MSB position");
    }

    /// Test: Memory expansion tracking
    #[test]
    fn test_memory_expansion() {
        // PUSH1 0x42, PUSH1 64, MSTORE, MSIZE, STOP
        // Store at offset 64, which requires 96 bytes (next multiple of 32 after 64+32)
        let bytecode = [0x60, 0x42, 0x60, 0x40, 0x52, 0x59, 0x00];

        let compiler = JitCompiler::new();
        let code = compiler.compile(&bytecode).expect("Failed to compile");

        let mut stack_values: Box<[U256; STACK_LIMIT]> =
            vec![U256::zero(); STACK_LIMIT].into_boxed_slice().try_into().unwrap();

        let mut memory = vec![0u8; 128];

        let mut ctx = JitContext {
            stack_values: stack_values.as_mut_ptr(),
            stack_offset: STACK_LIMIT,
            gas_remaining: 1000,
            memory_ptr: memory.as_mut_ptr(),
            memory_size: 0,
            memory_capacity: memory.len(),
            pc: 0,
            bytecode: bytecode.as_ptr(),
            bytecode_len: bytecode.len(),
            jump_table: std::ptr::null(),
            vm_ptr: std::ptr::null_mut(),
            exit_reason: 0,
            return_offset: 0,
            return_size: 0,
            push_value: U256::zero(),
            address: [0x11; 20],
            caller: [0x22; 20],
            callvalue: U256::from(1000u64),
            calldata_ptr: std::ptr::null(),
            calldata_len: 0,
            jmp_buf: Default::default(),
            exit_callback: None,
        };

        let exit_reason = unsafe { compiler::execute_jit(&code, &mut ctx) };

        assert_eq!(exit_reason, JitExitReason::Stop);
        assert_eq!(ctx.stack_offset, STACK_LIMIT - 1, "Should have one value on stack (MSIZE result)");
        // Memory size should be 96 (64 + 32 = 96, next 32-byte boundary)
        assert_eq!(ctx.memory_size, 96, "Memory size should be 96 after MSTORE at offset 64");
        assert_eq!(stack_values[ctx.stack_offset], U256::from(96u64), "MSIZE should return 96");
    }
}
