//! BridgeCircuit — AppCircuit implementation for the Bridge guest program.
//!
//! Bridge has NO app-specific operations. All bridge functionality
//! (deposits, withdrawals, ETH transfers) is handled by the common
//! `execute_app_circuit` engine.
//!
//! This makes it the fastest-proving guest program: only common
//! transaction types are processed, with no additional circuit logic.

use ethrex_common::types::{Log, Transaction};

use crate::common::app_execution::{AppCircuit, AppCircuitError, AppOperation, OperationResult};
use crate::common::app_state::AppState;

/// Bridge circuit — no app-specific operations.
pub struct BridgeCircuit;

impl AppCircuit for BridgeCircuit {
    fn classify_tx(&self, _tx: &Transaction) -> Result<AppOperation, AppCircuitError> {
        // Bridge has no app-specific operations.
        // All transactions are handled by the common engine:
        // - Privileged (deposits) → handle_privileged_tx()
        // - To bridge L2 (withdrawals) → handle_withdrawal()
        // - ETH transfers → handle_eth_transfer()
        // - System calls → handle_system_call()
        Err(AppCircuitError::UnknownTransaction)
    }

    fn execute_operation(
        &self,
        _state: &mut AppState,
        _from: ethrex_common::Address,
        _op: &AppOperation,
    ) -> Result<OperationResult, AppCircuitError> {
        // Never called since classify_tx always returns Err.
        Err(AppCircuitError::UnknownTransaction)
    }

    fn gas_cost(&self, _op: &AppOperation) -> u64 {
        0
    }

    fn generate_logs(
        &self,
        _from: ethrex_common::Address,
        _op: &AppOperation,
        _result: &OperationResult,
    ) -> Vec<Log> {
        vec![]
    }
}
