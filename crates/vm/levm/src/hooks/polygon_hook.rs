use crate::{
    errors::{ContextResult, VMError},
    hooks::{
        default_hook::{
            DefaultHook, compute_actual_gas_used, compute_gas_refunded,
            delete_self_destruct_accounts, refund_sender, undo_value_transfer,
        },
        hook::Hook,
    },
    vm::VM,
};

/// Hook for Polygon PoS execution.
///
/// Reuses DefaultHook's `prepare_execution` (same sender deduction, validation, etc.)
/// but skips `pay_coinbase()` in `finalize_execution` because Polygon uses deferred
/// fee distribution: fees are accumulated across all transactions and applied after
/// all transactions have been executed.
pub struct PolygonHook;

impl Hook for PolygonHook {
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), VMError> {
        // Same validation and upfront deduction as L1
        DefaultHook.prepare_execution(vm)
    }

    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        ctx_result: &mut ContextResult,
    ) -> Result<(), VMError> {
        if !ctx_result.is_success() {
            undo_value_transfer(vm)?;
        }

        let gas_used_pre_refund = ctx_result.gas_used;
        let gas_refunded: u64 = compute_gas_refunded(vm, ctx_result)?;
        let gas_spent = compute_actual_gas_used(vm, gas_refunded, gas_used_pre_refund)?;

        refund_sender(vm, ctx_result, gas_refunded, gas_spent, gas_used_pre_refund)?;

        // SKIP pay_coinbase() — fees are deferred on Polygon.
        // Base fee and tips are accumulated at the block level and distributed
        // after all transactions have been executed.

        delete_self_destruct_accounts(vm)?;

        Ok(())
    }
}
