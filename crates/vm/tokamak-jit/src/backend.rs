//! JIT backend — high-level API for compiling and executing EVM bytecode.
//!
//! Combines the compiler, adapter, and LEVM cache into a single entry point
//! for the Tokamak JIT system.

use ethrex_common::types::{Code, Fork};
use ethrex_levm::call_frame::CallFrame;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::Environment;
use ethrex_levm::jit::{
    analyzer::analyze_bytecode,
    cache::CodeCache,
    dispatch::JitBackend,
    optimizer,
    types::{AnalyzedBytecode, JitConfig, JitOutcome, JitResumeState, SubCallResult},
};
use ethrex_levm::vm::Substate;

use crate::compiler::TokamakCompiler;
use crate::error::JitError;

/// High-level JIT backend wrapping revmc compilation and execution.
#[derive(Debug)]
pub struct RevmcBackend {
    config: JitConfig,
}

impl RevmcBackend {
    /// Create a new backend with default configuration.
    pub fn new() -> Self {
        Self {
            config: JitConfig::default(),
        }
    }

    /// Create a new backend with custom configuration.
    pub fn with_config(config: JitConfig) -> Self {
        Self { config }
    }

    /// Analyze and compile bytecode for a specific fork, inserting the result into the cache.
    ///
    /// Returns `Ok(())` on success. The compiled code is stored in `cache`
    /// and can be retrieved via `cache.get(&(code.hash, fork))`.
    pub fn compile_and_cache(
        &self,
        code: &Code,
        fork: Fork,
        cache: &CodeCache,
    ) -> Result<(), JitError> {
        // Check bytecode size limit
        if self.config.is_bytecode_oversized(code.bytecode.len()) {
            return Err(JitError::BytecodeTooLarge {
                size: code.bytecode.len(),
                max: self.config.max_bytecode_size,
            });
        }

        // Skip empty bytecodes
        if code.bytecode.is_empty() {
            return Ok(());
        }

        // Analyze bytecode
        let analyzed =
            analyze_bytecode(code.bytecode.clone(), code.hash, code.jump_targets.clone());

        // Apply constant folding optimization before compilation
        let (analyzed, opt_stats) = optimizer::optimize(analyzed);
        if opt_stats.patterns_folded > 0 {
            tracing::info!(
                hash = %code.hash,
                patterns_folded = opt_stats.patterns_folded,
                opcodes_eliminated = opt_stats.opcodes_eliminated,
                "Bytecode optimized before JIT compilation"
            );
        }

        // Log if bytecode has external calls (used for metrics, no longer a gate)
        if analyzed.has_external_calls {
            tracing::info!(
                hash = %code.hash,
                "JIT compiling bytecode with external calls (CALL/CREATE resume enabled)"
            );
        }

        // Compile via revmc/LLVM for the target fork
        let compiled = TokamakCompiler::compile(&analyzed, fork)?;

        // Insert into cache with (hash, fork) key
        cache.insert((code.hash, fork), compiled);

        tracing::info!(
            hash = %code.hash,
            fork = ?fork,
            bytecode_size = code.bytecode.len(),
            basic_blocks = analyzed.basic_blocks.len(),
            "JIT compiled bytecode"
        );

        Ok(())
    }

    /// Analyze bytecode without compiling (for testing/inspection).
    pub fn analyze(&self, code: &Code) -> Result<AnalyzedBytecode, JitError> {
        if self.config.is_bytecode_oversized(code.bytecode.len()) {
            return Err(JitError::BytecodeTooLarge {
                size: code.bytecode.len(),
                max: self.config.max_bytecode_size,
            });
        }

        let analyzed =
            analyze_bytecode(code.bytecode.clone(), code.hash, code.jump_targets.clone());
        let (analyzed, _opt_stats) = optimizer::optimize(analyzed);
        Ok(analyzed)
    }
}

impl Default for RevmcBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl JitBackend for RevmcBackend {
    fn execute(
        &self,
        compiled: &ethrex_levm::jit::cache::CompiledCode,
        call_frame: &mut CallFrame,
        db: &mut GeneralizedDatabase,
        substate: &mut Substate,
        env: &Environment,
        storage_original_values: &mut ethrex_levm::jit::dispatch::StorageOriginalValues,
    ) -> Result<JitOutcome, String> {
        crate::execution::execute_jit(
            compiled,
            call_frame,
            db,
            substate,
            env,
            storage_original_values,
        )
        .map_err(|e| format!("{e}"))
    }

    fn execute_resume(
        &self,
        resume_state: JitResumeState,
        sub_result: SubCallResult,
        call_frame: &mut CallFrame,
        db: &mut GeneralizedDatabase,
        substate: &mut Substate,
        env: &Environment,
        storage_original_values: &mut ethrex_levm::jit::dispatch::StorageOriginalValues,
    ) -> Result<JitOutcome, String> {
        crate::execution::execute_jit_resume(
            resume_state,
            sub_result,
            call_frame,
            db,
            substate,
            env,
            storage_original_values,
        )
        .map_err(|e| format!("{e}"))
    }

    fn compile(&self, code: &Code, fork: Fork, cache: &CodeCache) -> Result<(), String> {
        self.compile_and_cache(code, fork, cache)
            .map_err(|e| format!("{e}"))
    }
}
