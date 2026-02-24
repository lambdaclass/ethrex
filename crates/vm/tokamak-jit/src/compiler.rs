//! revmc/LLVM compiler wrapper.
//!
//! Wraps the revmc `EvmCompiler` + `EvmLlvmBackend` pipeline, providing
//! a simplified API for compiling EVM bytecode to native code.

use crate::adapter::fork_to_spec_id;
use crate::error::JitError;
use ethrex_common::types::Fork;
use ethrex_levm::jit::cache::CompiledCode;
use ethrex_levm::jit::types::AnalyzedBytecode;

use revmc::{EvmCompiler, EvmLlvmBackend, OptimizationLevel};
use revmc_context::EvmCompilerFn;

/// JIT compiler backed by revmc + LLVM.
///
/// Each `TokamakCompiler` holds an LLVM context and can compile multiple
/// bytecodes. Compiled function pointers are returned as `CompiledCode`
/// for insertion into the global `CodeCache`.
pub struct TokamakCompiler {
    /// LLVM context — must outlive all compiled functions.
    /// We use `revmc_llvm::with_llvm_context` for thread-local usage,
    /// but for persistent compilation we store the context here.
    _marker: std::marker::PhantomData<()>,
}

impl TokamakCompiler {
    /// Compile analyzed bytecode into native code for a specific fork.
    ///
    /// Uses a thread-local LLVM context via `revmc_llvm::with_llvm_context`.
    /// The compiled function pointer is valid for the lifetime of the program
    /// (LLVM JIT memory is not freed until process exit in this PoC).
    pub fn compile(analyzed: &AnalyzedBytecode, fork: Fork) -> Result<CompiledCode, JitError> {
        let bytecode = analyzed.bytecode.as_ref();
        let hash_hex = format!("{:x}", analyzed.hash);
        let spec_id = fork_to_spec_id(fork);

        revmc::llvm::with_llvm_context(|cx| {
            let backend = EvmLlvmBackend::new(cx, false, OptimizationLevel::Aggressive)
                .map_err(|e| JitError::LlvmError(format!("backend init: {e}")))?;

            let mut compiler = EvmCompiler::new(backend);

            // SAFETY: The compiled function pointer is stored in CompiledCode
            // which is kept alive in the CodeCache. The LLVM JIT memory backing
            // the function is not freed (no `free_function` call in PoC).
            #[expect(unsafe_code)]
            let f: EvmCompilerFn = unsafe {
                compiler
                    .jit(&hash_hex, bytecode, spec_id)
                    .map_err(|e| JitError::CompilationFailed(format!("{e}")))?
            };

            // Extract the raw function pointer for type-erased storage in LEVM's cache.
            let raw_fn = f.into_inner();

            // SAFETY: The function pointer is valid executable JIT code produced by LLVM.
            // It conforms to the `RawEvmCompilerFn` calling convention.
            #[expect(unsafe_code, clippy::as_conversions)]
            let compiled = unsafe {
                CompiledCode::new(
                    raw_fn as *const (),
                    analyzed.bytecode.len(),
                    analyzed.basic_blocks.len(),
                    None, // func_id: not tracked yet (no persistent LLVM context)
                    analyzed.has_external_calls,
                )
            };

            Ok(compiled)
        })
    }
}

impl std::fmt::Debug for TokamakCompiler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokamakCompiler").finish()
    }
}
