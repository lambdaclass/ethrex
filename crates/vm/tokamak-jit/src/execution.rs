//! JIT execution bridge — runs JIT-compiled code through the revm interpreter.
//!
//! This module takes a `CompiledCode` function pointer (from the code cache),
//! builds the revm `Interpreter` and `Host` objects needed by revmc's calling
//! convention, executes the JIT function, and maps the result back to LEVM's
//! `JitOutcome`.
//!
//! # Safety
//!
//! This module uses `unsafe` to transmute the type-erased `CompiledCode` pointer
//! back to `EvmCompilerFn`. The safety invariant is maintained by the compilation
//! pipeline: only valid function pointers produced by revmc/LLVM are stored in
//! the code cache.

use bytes::Bytes;
use revm_bytecode::Bytecode;
use revm_interpreter::{
    CallInput, InputsImpl, Interpreter, InterpreterAction, SharedMemory, interpreter::ExtBytecode,
};
use revmc_context::EvmCompilerFn;

use crate::adapter::{fork_to_spec_id, levm_address_to_revm, revm_gas_to_levm};
use crate::error::JitError;
use crate::host::LevmHost;
use ethrex_levm::call_frame::CallFrame;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::Environment;
use ethrex_levm::jit::cache::CompiledCode;
use ethrex_levm::jit::types::JitOutcome;
use ethrex_levm::vm::Substate;

/// Execute JIT-compiled bytecode against LEVM state.
///
/// Follows the revmc calling convention: build an Interpreter with the contract's
/// bytecode and calldata, wrap LEVM state in a `LevmHost`, cast the compiled
/// function pointer to `EvmCompilerFn`, and invoke it.
///
/// # Errors
///
/// Returns `JitError` if the function pointer is null, the interpreter action
/// is unexpected, or host delegation fails.
pub fn execute_jit(
    compiled: &CompiledCode,
    call_frame: &mut CallFrame,
    db: &mut GeneralizedDatabase,
    substate: &mut Substate,
    env: &Environment,
    storage_original_values: &mut ethrex_levm::jit::dispatch::StorageOriginalValues,
) -> Result<JitOutcome, JitError> {
    let ptr = compiled.as_ptr();
    if ptr.is_null() {
        return Err(JitError::AdapterError(
            "null compiled code pointer".to_string(),
        ));
    }

    // Determine the SpecId from the environment's fork
    let spec_id = fork_to_spec_id(env.config.fork);

    // 1. Build revm Interpreter from LEVM CallFrame
    let bytecode_raw = Bytecode::new_raw(Bytes::copy_from_slice(&call_frame.bytecode.bytecode));
    let ext_bytecode = ExtBytecode::new(bytecode_raw);
    let input = InputsImpl {
        target_address: levm_address_to_revm(&call_frame.to),
        bytecode_address: None,
        caller_address: levm_address_to_revm(&call_frame.msg_sender),
        input: CallInput::Bytes(call_frame.calldata.clone()),
        call_value: crate::adapter::levm_u256_to_revm(&call_frame.msg_value),
    };

    #[expect(clippy::as_conversions, reason = "i64→u64 with clamping for gas")]
    let gas_limit = if call_frame.gas_remaining < 0 {
        0u64
    } else {
        call_frame.gas_remaining as u64
    };

    let mut interpreter = Interpreter::new(
        SharedMemory::new(),
        ext_bytecode,
        input,
        call_frame.is_static, // is_static — propagated from LEVM call frame
        spec_id,
        gas_limit,
    );

    // 2. Build Host wrapping LEVM state
    let mut host = LevmHost::new(db, substate, env, call_frame.code_address, storage_original_values);

    // 3. Cast CompiledCode pointer back to EvmCompilerFn
    //
    // SAFETY: The pointer was produced by revmc/LLVM via `TokamakCompiler::compile()`,
    // stored in `CompiledCode`, and conforms to the `RawEvmCompilerFn` calling
    // convention. The null check above ensures it's valid.
    #[expect(unsafe_code)]
    let f = unsafe { EvmCompilerFn::new(std::mem::transmute::<*const (), _>(ptr)) };

    // 4. Execute JIT-compiled code
    //
    // SAFETY: The function pointer is a valid `RawEvmCompilerFn` produced by the
    // revmc compiler. The interpreter and host are properly initialized above.
    #[expect(unsafe_code)]
    let action = unsafe { f.call_with_interpreter(&mut interpreter, &mut host) };

    // 5. Map InterpreterAction back to JitOutcome
    match action {
        InterpreterAction::Return(result) => {
            // Sync gas state back to LEVM call frame
            call_frame.gas_remaining = revm_gas_to_levm(&result.gas);

            // Sync gas refunds from revm interpreter to LEVM substate
            let refunded = result.gas.refunded();
            if refunded > 0 {
                #[expect(clippy::as_conversions, reason = "i64→u64 for gas refund")]
                let refunded_u64 = refunded as u64;
                host.substate.refunded_gas =
                    host.substate.refunded_gas.saturating_add(refunded_u64);
            }

            let gas_used = gas_limit.saturating_sub(result.gas.remaining());

            use revm_interpreter::InstructionResult;
            match result.result {
                InstructionResult::Stop | InstructionResult::Return => Ok(JitOutcome::Success {
                    gas_used,
                    output: result.output,
                }),
                InstructionResult::Revert => Ok(JitOutcome::Revert {
                    gas_used,
                    output: result.output,
                }),
                r => Ok(JitOutcome::Error(format!("JIT returned: {r:?}"))),
            }
        }
        InterpreterAction::NewFrame(_frame_input) => {
            // CALL/CREATE from JIT code — not supported yet.
            // The bytecode analyzer should have flagged this during compilation,
            // but if it reaches here, fall back to interpreter gracefully.
            Ok(JitOutcome::Error(
                "JIT encountered CALL/CREATE frame; falling back to interpreter".to_string(),
            ))
        }
    }
}
