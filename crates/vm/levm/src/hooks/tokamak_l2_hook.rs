use crate::{
    errors::{ContextResult, TxResult, VMError},
    hooks::{L2Hook, hook::Hook},
    vm::VM,
};
use ethrex_common::types::l2::tokamak_fee_config::TokamakFeeConfig;

/// Metadata recorded during proven execution for Tokamak L2.
#[derive(Debug, Clone, Default)]
pub struct TokamakExecutionMetadata {
    pub gas_used: u64,
    pub success: bool,
    pub state_change_count: u32,
}

/// Hook for Tokamak L2 execution.
///
/// Wraps the standard `L2Hook` via composition, delegating all standard L2
/// fee behavior. Adds Tokamak-specific proven execution metadata recording
/// when `config.proven_execution` is enabled.
pub struct TokamakL2Hook {
    inner: L2Hook,
    config: TokamakFeeConfig,
    metadata: Option<TokamakExecutionMetadata>,
}

impl TokamakL2Hook {
    pub fn new(config: TokamakFeeConfig) -> Self {
        Self {
            inner: L2Hook {
                fee_config: config.base,
            },
            config,
            metadata: None,
        }
    }

    /// Returns the recorded execution metadata, if any.
    pub fn metadata(&self) -> Option<&TokamakExecutionMetadata> {
        self.metadata.as_ref()
    }
}

impl Hook for TokamakL2Hook {
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), VMError> {
        if self.config.proven_execution {
            self.metadata = Some(TokamakExecutionMetadata::default());
        }
        self.inner.prepare_execution(vm)
    }

    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        report: &mut ContextResult,
    ) -> Result<(), VMError> {
        self.inner.finalize_execution(vm, report)?;

        if let Some(ref mut meta) = self.metadata {
            meta.gas_used = report.gas_used;
            meta.success = matches!(report.result, TxResult::Success);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::fee_config::FeeConfig;
    use ethrex_common::types::l2::tokamak_fee_config::JitPolicy;

    #[test]
    fn test_hook_creation() {
        let config = TokamakFeeConfig {
            base: FeeConfig::default(),
            proven_execution: false,
            jit_policy: JitPolicy::EnabledByDefault,
        };
        let hook = TokamakL2Hook::new(config);
        assert!(hook.metadata().is_none());
    }

    #[test]
    fn test_metadata_none_when_disabled() {
        let config = TokamakFeeConfig {
            base: FeeConfig::default(),
            proven_execution: false,
            jit_policy: JitPolicy::Disabled,
        };
        let hook = TokamakL2Hook::new(config);
        assert!(hook.metadata().is_none());
    }

    #[test]
    fn test_hook_wraps_l2hook() {
        let config = TokamakFeeConfig {
            base: FeeConfig::default(),
            proven_execution: true,
            jit_policy: JitPolicy::Tiered,
        };
        let hook = TokamakL2Hook::new(config);
        // Verify the inner L2Hook has the same base fee config
        assert!(hook.inner.fee_config.base_fee_vault.is_none());
        assert!(hook.inner.fee_config.operator_fee_config.is_none());
    }
}
