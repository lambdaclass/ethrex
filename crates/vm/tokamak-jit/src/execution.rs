//! JIT execution bridge — runs JIT-compiled code through the revm interpreter.
//!
//! This module takes a `CompiledCode` function pointer (from the code cache),
//! builds the revm `Interpreter` and `Host` objects needed by revmc's calling
//! convention, executes the JIT function, and maps the result back to LEVM's
//! `JitOutcome`.
//!
//! # Suspend/Resume
//!
//! When JIT code encounters a CALL/CREATE opcode, revmc suspends execution
//! by returning `InterpreterAction::NewFrame(FrameInput)`. We translate this
//! to `JitOutcome::Suspended`, passing the revm Interpreter (with stack/memory/
//! gas state preserved) back as opaque `JitResumeState`. After the caller
//! executes the sub-call, `execute_jit_resume` applies the sub-call result
//! and re-invokes the JIT function.
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
    CallInput, InputsImpl, Interpreter, InterpreterAction, SharedMemory,
    interpreter::ExtBytecode,
    interpreter_action::FrameInput,
    interpreter_types::{ReturnData, StackTr},
};
use revm_primitives::U256 as RevmU256;
use revmc_context::EvmCompilerFn;

use crate::adapter::{
    fork_to_spec_id, levm_address_to_revm, revm_address_to_levm, revm_gas_to_levm,
    revm_u256_to_levm,
};
use crate::error::JitError;
use crate::host::LevmHost;
use ethrex_levm::call_frame::CallFrame;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::Environment;
use ethrex_levm::jit::cache::CompiledCode;
use ethrex_levm::jit::types::{
    JitCallScheme, JitOutcome, JitResumeState, JitSubCall, SubCallResult,
};
use ethrex_levm::vm::Substate;

/// Internal resume state preserved across suspend/resume cycles.
/// Private to tokamak-jit; exposed to LEVM only as `JitResumeState(Box<dyn Any + Send>)`.
struct JitResumeStateInner {
    interpreter: Interpreter,
    compiled_fn: EvmCompilerFn,
    gas_limit: u64,
}

// SAFETY: Interpreter contains SharedMemory (Arc-backed) and other Send-safe types.
// EvmCompilerFn is a function pointer (inherently Send).
#[expect(unsafe_code)]
unsafe impl Send for JitResumeStateInner {}

/// Execute JIT-compiled bytecode against LEVM state (single step).
///
/// Returns `JitOutcome::Success`/`Revert` for terminal execution, or
/// `JitOutcome::Suspended` if JIT code hit a CALL/CREATE and needs
/// the caller to execute the sub-call.
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

    let spec_id = fork_to_spec_id(env.config.fork);

    // Build revm Interpreter from LEVM CallFrame
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
        call_frame.is_static,
        spec_id,
        gas_limit,
    );

    // Build Host wrapping LEVM state
    let mut host = LevmHost::new(
        db,
        substate,
        env,
        call_frame.code_address,
        storage_original_values,
    );

    // Cast CompiledCode pointer back to EvmCompilerFn
    //
    // SAFETY: The pointer was produced by revmc/LLVM via `TokamakCompiler::compile()`,
    // stored in `CompiledCode`, and conforms to the `RawEvmCompilerFn` calling
    // convention. The null check above ensures it's valid.
    #[expect(unsafe_code)]
    let f = unsafe { EvmCompilerFn::new(std::mem::transmute::<*const (), _>(ptr)) };

    // Execute JIT-compiled code (single step)
    //
    // SAFETY: The function pointer is a valid `RawEvmCompilerFn` produced by the
    // revmc compiler. The interpreter and host are properly initialized above.
    #[expect(unsafe_code)]
    let action = unsafe { f.call_with_interpreter(&mut interpreter, &mut host) };

    handle_interpreter_action(action, interpreter, f, gas_limit, call_frame, host)
}

/// Resume JIT execution after a sub-call completes.
///
/// Downcasts the opaque `JitResumeState`, applies the sub-call result to
/// the revm interpreter's stack/memory, and re-invokes the JIT function.
pub fn execute_jit_resume(
    resume_state: JitResumeState,
    sub_result: SubCallResult,
    call_frame: &mut CallFrame,
    db: &mut GeneralizedDatabase,
    substate: &mut Substate,
    env: &Environment,
    storage_original_values: &mut ethrex_levm::jit::dispatch::StorageOriginalValues,
) -> Result<JitOutcome, JitError> {
    // Downcast the opaque state
    let inner = resume_state
        .0
        .downcast::<JitResumeStateInner>()
        .map_err(|_| JitError::AdapterError("invalid JitResumeState type".to_string()))?;

    let mut interpreter = inner.interpreter;
    let f = inner.compiled_fn;
    let gas_limit = inner.gas_limit;

    // Apply sub-call result to interpreter stack
    apply_subcall_result(&mut interpreter, &sub_result);

    // Build new Host for this invocation (scoped borrows)
    let mut host = LevmHost::new(
        db,
        substate,
        env,
        call_frame.code_address,
        storage_original_values,
    );

    // Re-invoke JIT function (interpreter has resume_at set by revmc)
    //
    // SAFETY: Same function pointer, interpreter preserves stack/memory/gas state.
    #[expect(unsafe_code)]
    let action = unsafe { f.call_with_interpreter(&mut interpreter, &mut host) };

    handle_interpreter_action(action, interpreter, f, gas_limit, call_frame, host)
}

/// Process the `InterpreterAction` returned by revmc, producing a `JitOutcome`.
///
/// On `Return` → terminal `Success`/`Revert`/`Error`.
/// On `NewFrame` → `Suspended` with resume state and translated sub-call.
fn handle_interpreter_action(
    action: InterpreterAction,
    interpreter: Interpreter,
    compiled_fn: EvmCompilerFn,
    gas_limit: u64,
    call_frame: &mut CallFrame,
    host: LevmHost<'_>,
) -> Result<JitOutcome, JitError> {
    match action {
        InterpreterAction::Return(result) => {
            // Sync gas state back to LEVM call frame
            call_frame.gas_remaining = revm_gas_to_levm(&result.gas);

            // Sync gas refunds from revm interpreter to LEVM substate.
            let refunded = result.gas.refunded();
            if let Ok(refunded_u64) = u64::try_from(refunded) {
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
        InterpreterAction::NewFrame(frame_input) => {
            // Translate revm FrameInput to LEVM JitSubCall
            let sub_call = translate_frame_input(frame_input)?;

            // Pack interpreter + fn into opaque resume state
            let resume_state = JitResumeState(Box::new(JitResumeStateInner {
                interpreter,
                compiled_fn,
                gas_limit,
            }));

            Ok(JitOutcome::Suspended {
                resume_state,
                sub_call,
            })
        }
    }
}

/// Translate a revm `FrameInput` into an LEVM `JitSubCall`.
fn translate_frame_input(frame_input: FrameInput) -> Result<JitSubCall, JitError> {
    match frame_input {
        FrameInput::Call(call_inputs) => {
            use revm_interpreter::interpreter_action::CallScheme;

            let scheme = match call_inputs.scheme {
                CallScheme::Call => JitCallScheme::Call,
                CallScheme::CallCode => JitCallScheme::CallCode,
                CallScheme::DelegateCall => JitCallScheme::DelegateCall,
                CallScheme::StaticCall => JitCallScheme::StaticCall,
            };

            let value = revm_u256_to_levm(&call_inputs.value.get());

            // Extract calldata — for JIT calls it should be Bytes variant
            let calldata = match &call_inputs.input {
                CallInput::Bytes(b) => b.clone(),
                CallInput::SharedBuffer(_) => {
                    // SharedBuffer shouldn't happen in JIT context
                    Bytes::new()
                }
            };

            let return_offset = call_inputs.return_memory_offset.start;
            let return_size = call_inputs.return_memory_offset.len();

            let is_static =
                call_inputs.is_static || matches!(call_inputs.scheme, CallScheme::StaticCall);

            Ok(JitSubCall::Call {
                gas_limit: call_inputs.gas_limit,
                caller: revm_address_to_levm(&call_inputs.caller),
                target: revm_address_to_levm(&call_inputs.target_address),
                code_address: revm_address_to_levm(&call_inputs.bytecode_address),
                value,
                calldata,
                is_static,
                scheme,
                return_offset,
                return_size,
            })
        }
        FrameInput::Create(create_inputs) => {
            use revm_context_interface::CreateScheme;

            let salt = match create_inputs.scheme() {
                CreateScheme::Create2 { salt } => Some(revm_u256_to_levm(&salt)),
                _ => None,
            };

            Ok(JitSubCall::Create {
                gas_limit: create_inputs.gas_limit(),
                caller: revm_address_to_levm(&create_inputs.caller()),
                value: revm_u256_to_levm(&create_inputs.value()),
                init_code: create_inputs.init_code().clone(),
                salt,
            })
        }
        FrameInput::Empty => Err(JitError::AdapterError(
            "unexpected empty FrameInput from JIT".to_string(),
        )),
    }
}

/// Apply a sub-call result to the revm interpreter before resume.
///
/// Pushes the success/failure value onto the revm stack and sets return_data.
/// For CREATE success, pushes the created address instead of 1.
fn apply_subcall_result(interpreter: &mut Interpreter, sub_result: &SubCallResult) {
    // Push return value onto the revm stack
    let return_value = if sub_result.success {
        match sub_result.created_address {
            // CREATE success: push the created address as U256
            Some(addr) => {
                let addr_bytes = addr.as_bytes();
                RevmU256::from_be_slice(addr_bytes)
            }
            // CALL success: push 1
            None => RevmU256::from(1u64),
        }
    } else {
        // Failure: push 0
        RevmU256::ZERO
    };

    // Push onto revm stack using the StackTr trait.
    // revmc's compiled code accounts for CALL/CREATE stack effects, so there
    // is guaranteed space for this push.
    let _ok = interpreter.stack.push(return_value);

    // Set return_data for RETURNDATASIZE/RETURNDATACOPY opcodes
    interpreter
        .return_data
        .set_buffer(sub_result.output.clone());
}
