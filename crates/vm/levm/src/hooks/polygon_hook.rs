use crate::{
    errors::ContextResult,
    errors::{InternalError, VMError},
    hooks::{
        default_hook::{
            DefaultHook, compute_gas_refunded, delete_self_destruct_accounts, refund_sender,
            undo_value_transfer,
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
///
/// Also uses Polygon-specific gas computation: Bor does NOT implement EIP-7623
/// (calldata floor gas), so the gas_spent is simply gas_used minus refund,
/// without the `max(exec_gas, floor_gas)` enforcement that Ethereum Prague adds.
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

        // Polygon gas computation: simple subtraction without EIP-7623 floor.
        // Bor doesn't implement EIP-7623 (calldata floor gas from Ethereum Pectra),
        // so gas_spent = gas_used - refund, no floor enforcement.
        let gas_spent = gas_used_pre_refund
            .checked_sub(gas_refunded)
            .ok_or(InternalError::Underflow)?;

        refund_sender(vm, ctx_result, gas_refunded, gas_spent, gas_used_pre_refund)?;

        // SKIP pay_coinbase() — fees are deferred on Polygon.
        // Base fee and tips are accumulated at the block level and distributed
        // after all transactions have been executed.

        delete_self_destruct_accounts(vm)?;

        Ok(())
    }
}
