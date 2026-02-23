//! JIT backend — high-level API for compiling and executing EVM bytecode.
//!
//! Combines the compiler, adapter, and LEVM cache into a single entry point
//! for the Tokamak JIT system.

use bytes::Bytes;
use ethrex_common::types::Code;
use ethrex_levm::call_frame::CallFrame;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::Environment;
use ethrex_levm::jit::{
    analyzer::analyze_bytecode,
    cache::{CodeCache, CompiledCode},
    dispatch::JitBackend,
    types::{AnalyzedBytecode, JitConfig, JitOutcome},
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

    /// Analyze and compile bytecode, inserting the result into the cache.
    ///
    /// Returns `Ok(())` on success. The compiled code is stored in `cache`
    /// and can be retrieved via `cache.get(&code.hash)`.
    pub fn compile_and_cache(&self, code: &Code, cache: &CodeCache) -> Result<(), JitError> {
        // Check bytecode size limit
        if code.bytecode.len() > self.config.max_bytecode_size {
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

        // Compile via revmc/LLVM
        let compiled = TokamakCompiler::compile(&analyzed)?;

        // Insert into cache
        cache.insert(code.hash, compiled);

        tracing::info!(
            hash = %code.hash,
            bytecode_size = code.bytecode.len(),
            basic_blocks = analyzed.basic_blocks.len(),
            "JIT compiled bytecode"
        );

        Ok(())
    }

    /// Analyze bytecode without compiling (for testing/inspection).
    pub fn analyze(&self, code: &Code) -> Result<AnalyzedBytecode, JitError> {
        if code.bytecode.len() > self.config.max_bytecode_size {
            return Err(JitError::BytecodeTooLarge {
                size: code.bytecode.len(),
                max: self.config.max_bytecode_size,
            });
        }

        Ok(analyze_bytecode(
            code.bytecode.clone(),
            code.hash,
            code.jump_targets.clone(),
        ))
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
        compiled: &CompiledCode,
        call_frame: &mut CallFrame,
        db: &mut GeneralizedDatabase,
        substate: &mut Substate,
        env: &Environment,
    ) -> Result<JitOutcome, String> {
        crate::execution::execute_jit(compiled, call_frame, db, substate, env)
            .map_err(|e| format!("{e}"))
    }
}
