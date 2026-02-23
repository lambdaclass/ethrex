//! Validation mode — dual execution for consensus safety.
//!
//! In validation mode, every JIT execution is followed by an interpreter
//! execution on the same input. Results are compared; mismatches trigger
//! cache invalidation and a fallback to the interpreter result.
//!
//! This is mandatory during PoC (Phase 2) and recommended during Phase 3
//! until confidence in the JIT's correctness is established.

use ethrex_levm::jit::types::JitOutcome;

use crate::error::JitError;

/// Compare JIT and interpreter outcomes for validation.
///
/// Returns `Ok(())` if the outcomes match (gas_used and output identical),
/// or `Err(JitError::ValidationMismatch)` with details if they diverge.
pub fn validate_outcomes(
    jit_outcome: &JitOutcome,
    interp_gas_used: u64,
    interp_output: &[u8],
    interp_success: bool,
) -> Result<(), JitError> {
    match jit_outcome {
        JitOutcome::Success {
            gas_used, output, ..
        } => {
            if !interp_success {
                return Err(JitError::ValidationMismatch {
                    reason: format!(
                        "JIT succeeded but interpreter reverted (jit_gas={gas_used}, interp_gas={interp_gas_used})"
                    ),
                });
            }
            if *gas_used != interp_gas_used {
                return Err(JitError::ValidationMismatch {
                    reason: format!("gas mismatch: JIT={gas_used}, interpreter={interp_gas_used}"),
                });
            }
            if output.as_ref() != interp_output {
                return Err(JitError::ValidationMismatch {
                    reason: format!(
                        "output mismatch: JIT={} bytes, interpreter={} bytes",
                        output.len(),
                        interp_output.len()
                    ),
                });
            }
        }
        JitOutcome::Revert {
            gas_used, output, ..
        } => {
            if interp_success {
                return Err(JitError::ValidationMismatch {
                    reason: format!(
                        "JIT reverted but interpreter succeeded (jit_gas={gas_used}, interp_gas={interp_gas_used})"
                    ),
                });
            }
            if *gas_used != interp_gas_used {
                return Err(JitError::ValidationMismatch {
                    reason: format!(
                        "revert gas mismatch: JIT={gas_used}, interpreter={interp_gas_used}"
                    ),
                });
            }
            if output.as_ref() != interp_output {
                return Err(JitError::ValidationMismatch {
                    reason: format!(
                        "revert output mismatch: JIT={} bytes, interpreter={} bytes",
                        output.len(),
                        interp_output.len()
                    ),
                });
            }
        }
        JitOutcome::NotCompiled => {
            return Err(JitError::ValidationMismatch {
                reason: "JIT returned NotCompiled during validation".to_string(),
            });
        }
        JitOutcome::Error(msg) => {
            return Err(JitError::ValidationMismatch {
                reason: format!("JIT error during validation: {msg}"),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn test_matching_success() {
        let outcome = JitOutcome::Success {
            gas_used: 100,
            output: Bytes::from_static(b"\x01"),
        };
        assert!(validate_outcomes(&outcome, 100, b"\x01", true).is_ok());
    }

    #[test]
    fn test_gas_mismatch() {
        let outcome = JitOutcome::Success {
            gas_used: 100,
            output: Bytes::new(),
        };
        let err = validate_outcomes(&outcome, 200, b"", true).unwrap_err();
        match err {
            JitError::ValidationMismatch { reason } => {
                assert!(reason.contains("gas mismatch"));
            }
            _ => panic!("expected ValidationMismatch"),
        }
    }

    #[test]
    fn test_success_vs_revert_mismatch() {
        let outcome = JitOutcome::Success {
            gas_used: 100,
            output: Bytes::new(),
        };
        let err = validate_outcomes(&outcome, 100, b"", false).unwrap_err();
        match err {
            JitError::ValidationMismatch { reason } => {
                assert!(reason.contains("interpreter reverted"));
            }
            _ => panic!("expected ValidationMismatch"),
        }
    }
}
