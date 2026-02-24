//! Dual-execution validation for JIT-compiled code.
//!
//! When validation mode is active, the VM runs both JIT and interpreter on the
//! same input state and compares their outcomes. Mismatches trigger cache
//! invalidation and fallback to the interpreter result.

use crate::errors::{ContextResult, TxResult};

/// Result of comparing JIT execution against interpreter execution.
#[derive(Debug)]
pub enum DualExecutionResult {
    /// JIT and interpreter produced identical results.
    Match,
    /// JIT and interpreter diverged.
    Mismatch { reason: String },
}

/// Compare a JIT execution outcome against an interpreter execution outcome.
///
/// Checks status, gas_used, output bytes, and logs (via the substate).
/// The `jit_logs` and `interp_logs` are passed separately since they come
/// from different substate snapshots.
pub fn validate_dual_execution(
    jit_result: &ContextResult,
    interp_result: &ContextResult,
    jit_refunded_gas: u64,
    interp_refunded_gas: u64,
    jit_logs: &[ethrex_common::types::Log],
    interp_logs: &[ethrex_common::types::Log],
) -> DualExecutionResult {
    // 1. Compare status (success vs revert)
    let jit_success = matches!(jit_result.result, TxResult::Success);
    let interp_success = matches!(interp_result.result, TxResult::Success);
    if jit_success != interp_success {
        return DualExecutionResult::Mismatch {
            reason: format!(
                "status mismatch: JIT={}, interpreter={}",
                if jit_success { "success" } else { "revert" },
                if interp_success { "success" } else { "revert" },
            ),
        };
    }

    // 2. Compare gas_used
    if jit_result.gas_used != interp_result.gas_used {
        return DualExecutionResult::Mismatch {
            reason: format!(
                "gas_used mismatch: JIT={}, interpreter={}",
                jit_result.gas_used, interp_result.gas_used,
            ),
        };
    }

    // 3. Compare output bytes
    if jit_result.output != interp_result.output {
        return DualExecutionResult::Mismatch {
            reason: format!(
                "output mismatch: JIT len={}, interpreter len={}",
                jit_result.output.len(),
                interp_result.output.len(),
            ),
        };
    }

    // 4. Compare refunded gas
    if jit_refunded_gas != interp_refunded_gas {
        return DualExecutionResult::Mismatch {
            reason: format!(
                "refunded_gas mismatch: JIT={jit_refunded_gas}, interpreter={interp_refunded_gas}",
            ),
        };
    }

    // 5. Compare logs (count + ordered equality)
    if jit_logs.len() != interp_logs.len() {
        return DualExecutionResult::Mismatch {
            reason: format!(
                "log count mismatch: JIT={}, interpreter={}",
                jit_logs.len(),
                interp_logs.len(),
            ),
        };
    }
    for (i, (jit_log, interp_log)) in jit_logs.iter().zip(interp_logs.iter()).enumerate() {
        if jit_log != interp_log {
            return DualExecutionResult::Mismatch {
                reason: format!("log mismatch at index {i}"),
            };
        }
    }

    DualExecutionResult::Match
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::types::Log;
    use ethrex_common::{Address, H256};

    fn success_result(gas_used: u64, output: &[u8]) -> ContextResult {
        ContextResult {
            result: TxResult::Success,
            gas_used,
            gas_spent: gas_used,
            output: Bytes::copy_from_slice(output),
        }
    }

    fn revert_result(gas_used: u64, output: &[u8]) -> ContextResult {
        use crate::errors::VMError;
        ContextResult {
            result: TxResult::Revert(VMError::RevertOpcode),
            gas_used,
            gas_spent: gas_used,
            output: Bytes::copy_from_slice(output),
        }
    }

    fn make_log(addr: Address, topics: Vec<H256>, data: Vec<u8>) -> Log {
        Log {
            address: addr,
            topics,
            data: Bytes::from(data),
        }
    }

    #[test]
    fn test_matching_success_outcomes() {
        let jit = success_result(21000, &[0x01, 0x02]);
        let interp = success_result(21000, &[0x01, 0x02]);
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[]);
        assert!(matches!(result, DualExecutionResult::Match));
    }

    #[test]
    fn test_gas_mismatch() {
        let jit = success_result(21000, &[]);
        let interp = success_result(21500, &[]);
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[]);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("gas_used"));
        }
    }

    #[test]
    fn test_output_mismatch() {
        let jit = success_result(21000, &[0x01]);
        let interp = success_result(21000, &[0x02]);
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[]);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("output"));
        }
    }

    #[test]
    fn test_status_mismatch_success_vs_revert() {
        let jit = success_result(21000, &[]);
        let interp = revert_result(21000, &[]);
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[]);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("status"));
        }
    }

    #[test]
    fn test_log_count_mismatch() {
        let jit = success_result(21000, &[]);
        let interp = success_result(21000, &[]);
        let log = make_log(Address::zero(), vec![], vec![0x42]);
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[log], &[]);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("log count"));
        }
    }

    #[test]
    fn test_refunded_gas_mismatch() {
        let jit = success_result(21000, &[]);
        let interp = success_result(21000, &[]);
        let result = validate_dual_execution(&jit, &interp, 100, 200, &[], &[]);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("refunded_gas"));
        }
    }

    #[test]
    fn test_matching_with_logs() {
        let jit = success_result(30000, &[0xAA]);
        let interp = success_result(30000, &[0xAA]);
        let log1 = make_log(Address::zero(), vec![H256::zero()], vec![1, 2, 3]);
        let log2 = make_log(Address::zero(), vec![H256::zero()], vec![1, 2, 3]);
        let result = validate_dual_execution(&jit, &interp, 50, 50, &[log1], &[log2]);
        assert!(matches!(result, DualExecutionResult::Match));
    }

    #[test]
    fn test_log_content_mismatch() {
        let jit = success_result(30000, &[]);
        let interp = success_result(30000, &[]);
        let jit_log = make_log(Address::zero(), vec![], vec![1]);
        let interp_log = make_log(Address::zero(), vec![], vec![2]);
        let result =
            validate_dual_execution(&jit, &interp, 0, 0, &[jit_log], &[interp_log]);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("log mismatch at index"));
        }
    }
}
