use serde::{Deserialize, Serialize};

use super::fee_config::FeeConfig;

/// JIT compilation policy for Tokamak L2 execution.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JitPolicy {
    /// JIT compilation is enabled by default for eligible contracts.
    #[default]
    EnabledByDefault,
    /// JIT compilation is disabled; interpreter-only execution.
    Disabled,
    /// Tiered compilation: interpret first, JIT-compile after threshold.
    Tiered,
}

/// Fee and execution configuration for Tokamak L2.
///
/// Wraps the standard `FeeConfig` (reusing its serialization) and adds
/// Tokamak-specific fields for proven execution and JIT policy.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub struct TokamakFeeConfig {
    /// Standard L2 fee configuration (base fee vault, operator fee, L1 fee).
    pub base: FeeConfig,
    /// Whether proven execution metadata should be recorded.
    pub proven_execution: bool,
    /// JIT compilation policy.
    pub jit_policy: JitPolicy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let config = TokamakFeeConfig::default();
        assert!(!config.proven_execution);
        assert!(matches!(config.jit_policy, JitPolicy::EnabledByDefault));
        assert!(config.base.base_fee_vault.is_none());
        assert!(config.base.operator_fee_config.is_none());
        assert!(config.base.l1_fee_config.is_none());
    }

    #[test]
    fn test_serde_roundtrip() {
        let config = TokamakFeeConfig {
            base: FeeConfig::default(),
            proven_execution: true,
            jit_policy: JitPolicy::Tiered,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let decoded: TokamakFeeConfig = serde_json::from_str(&json).expect("deserialize");
        assert!(decoded.proven_execution);
        assert!(matches!(decoded.jit_policy, JitPolicy::Tiered));
    }

    #[test]
    fn test_jit_policy_variants() {
        for (policy, expected) in [
            (JitPolicy::EnabledByDefault, "\"EnabledByDefault\""),
            (JitPolicy::Disabled, "\"Disabled\""),
            (JitPolicy::Tiered, "\"Tiered\""),
        ] {
            let json = serde_json::to_string(&policy).expect("serialize");
            assert_eq!(json, expected);
            let decoded: JitPolicy = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(decoded, policy);
        }
    }

    #[test]
    fn test_with_full_base_config() {
        use crate::types::fee_config::{L1FeeConfig, OperatorFeeConfig};
        use ethereum_types::H160;

        let config = TokamakFeeConfig {
            base: FeeConfig {
                base_fee_vault: Some(H160::from_low_u64_be(1)),
                operator_fee_config: Some(OperatorFeeConfig {
                    operator_fee_vault: H160::from_low_u64_be(2),
                    operator_fee_per_gas: 100,
                }),
                l1_fee_config: Some(L1FeeConfig {
                    l1_fee_vault: H160::from_low_u64_be(3),
                    l1_fee_per_blob_gas: 200,
                }),
            },
            proven_execution: true,
            jit_policy: JitPolicy::EnabledByDefault,
        };

        let json = serde_json::to_string(&config).expect("serialize");
        let decoded: TokamakFeeConfig = serde_json::from_str(&json).expect("deserialize");
        assert!(decoded.proven_execution);
        assert!(decoded.base.base_fee_vault.is_some());
        assert!(decoded.base.operator_fee_config.is_some());
        assert!(decoded.base.l1_fee_config.is_some());
    }
}
