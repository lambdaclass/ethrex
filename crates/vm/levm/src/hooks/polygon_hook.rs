use bytes::Bytes;
use ethrex_common::types::Log;
use ethrex_common::{Address, H160, H256, U256};

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

/// The Polygon state receiver contract that emits fee/transfer logs.
const BOR_FEE_CONTRACT: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x10, 0x10,
]);

/// Topic hash for LogFeeTransfer events.
/// keccak256("LogFeeTransfer(address,address,address,uint256,uint256,uint256,uint256,uint256)")
const LOG_FEE_TRANSFER_TOPIC: H256 = H256([
    0x4d, 0xfe, 0x1b, 0xbb, 0xcf, 0x07, 0x7d, 0xdc, 0x3e, 0x01, 0x29, 0x1e, 0xea, 0x2d, 0x5c, 0x70,
    0xc2, 0xb4, 0x22, 0xb4, 0x15, 0xd9, 0x56, 0x45, 0xb9, 0xad, 0xcf, 0xd6, 0x78, 0xcb, 0x1d, 0x63,
]);

/// Hook for Polygon PoS execution.
///
/// Reuses DefaultHook's `prepare_execution` (same sender deduction, validation, etc.)
/// but replaces `pay_coinbase()` with Polygon-specific fee distribution:
/// - Tips (priority fees) go to BorConfig coinbase in real-time per tx
/// - Base fees go to burnt contract in real-time per tx
///
/// Also uses Polygon-specific gas computation: Bor does NOT implement EIP-7623
/// (calldata floor gas), so the gas_spent is simply gas_used minus refund,
/// without the `max(exec_gas, floor_gas)` enforcement that Ethereum Prague adds.
///
/// Emits a `LogFeeTransfer` log after every transaction (matching Bor behavior).
pub struct PolygonHook {
    /// Sender balance captured BEFORE gas deduction (buyGas).
    sender_balance_before: U256,
    /// Coinbase balance captured BEFORE any fees are paid.
    coinbase_balance_before: U256,
}

impl Default for PolygonHook {
    fn default() -> Self {
        Self {
            sender_balance_before: U256::zero(),
            coinbase_balance_before: U256::zero(),
        }
    }
}

impl Hook for PolygonHook {
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), VMError> {
        // Capture balances BEFORE gas deduction (Bor captures these at the top of execute())
        let sender = vm.env.origin;
        let coinbase = vm.env.coinbase;
        self.sender_balance_before = vm.db.get_account(sender)?.info.balance;
        self.coinbase_balance_before = vm.db.get_account(coinbase)?.info.balance;

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

        // Pay tip to coinbase in real-time (matching Bor's state_transition.go).
        // Base fee distribution is deferred — handled in execute_block after all txs.
        let effective_tip = vm
            .env
            .gas_price
            .checked_sub(vm.env.base_fee_per_gas)
            .unwrap_or(U256::zero());
        let tip_amount = U256::from(gas_spent)
            .checked_mul(effective_tip)
            .ok_or(InternalError::Overflow)?;

        if !tip_amount.is_zero() {
            vm.increase_account_balance(vm.env.coinbase, tip_amount)?;
        }

        // Emit LogFeeTransfer log (Bor adds this after every transaction).
        // amount = gas_spent * effective_tip
        if !tip_amount.is_zero() {
            // Bor computes synthetic output values: output1 = input1 - amount, output2 = input2 + amount
            // These do NOT reflect actual state — they model the tip movement from pre-tx balances.
            let output1 = self.sender_balance_before.saturating_sub(tip_amount);
            let output2 = self.coinbase_balance_before.saturating_add(tip_amount);

            let fee_log = build_fee_transfer_log(
                vm.env.origin,
                vm.env.coinbase,
                tip_amount,
                self.sender_balance_before,
                self.coinbase_balance_before,
                output1,
                output2,
            );
            vm.substate.add_log(fee_log);
        }

        delete_self_destruct_accounts(vm)?;

        Ok(())
    }
}

/// Builds a Bor LogFeeTransfer log matching `core/bor_fee_log.go:AddFeeTransferLog`.
fn build_fee_transfer_log(
    sender: Address,
    coinbase: Address,
    amount: U256,
    input1: U256,
    input2: U256,
    output1: U256,
    output2: U256,
) -> Log {
    build_transfer_log(
        LOG_FEE_TRANSFER_TOPIC,
        sender,
        coinbase,
        amount,
        input1,
        input2,
        output1,
        output2,
    )
}

/// Topic hash for LogTransfer events (native value transfers).
/// keccak256("LogTransfer(address,address,address,uint256,uint256,uint256,uint256,uint256)")
const LOG_TRANSFER_TOPIC: H256 = H256([
    0xe6, 0x49, 0x7e, 0x3e, 0xe5, 0x48, 0xa3, 0x37, 0x21, 0x36, 0xaf, 0x2f, 0xcb, 0x06, 0x96, 0xdb,
    0x31, 0xfc, 0x6c, 0xf2, 0x02, 0x60, 0x70, 0x76, 0x45, 0x06, 0x8b, 0xd3, 0xfe, 0x97, 0xf3, 0xc4,
]);

/// Builds a Bor LogTransfer log for native value transfers.
/// Matches `core/bor_fee_log.go:AddTransferLog`.
///
/// Call this AFTER the balance transfer has been applied.
/// Reconstructs before-balances from after-balances and the transfer amount.
pub fn build_value_transfer_log(
    sender: Address,
    recipient: Address,
    amount: U256,
    sender_balance_after: U256,
    recipient_balance_after: U256,
) -> Log {
    // Reconstruct pre-transfer balances
    let (input1, input2) = if sender == recipient {
        // Self-transfer: balances didn't change
        (sender_balance_after, recipient_balance_after)
    } else {
        (
            sender_balance_after.saturating_add(amount),
            recipient_balance_after.saturating_sub(amount),
        )
    };

    build_transfer_log(
        LOG_TRANSFER_TOPIC,
        sender,
        recipient,
        amount,
        input1,
        input2,
        sender_balance_after,
        recipient_balance_after,
    )
}

/// Shared log builder for both LogFeeTransfer and LogTransfer.
fn build_transfer_log(
    event_sig: H256,
    from: Address,
    to: Address,
    amount: U256,
    input1: U256,
    input2: U256,
    output1: U256,
    output2: U256,
) -> Log {
    let topics = vec![
        event_sig,
        address_to_h256(BOR_FEE_CONTRACT),
        address_to_h256(from),
        address_to_h256(to),
    ];

    let mut data = Vec::with_capacity(160);
    data.extend_from_slice(&amount.to_big_endian());
    data.extend_from_slice(&input1.to_big_endian());
    data.extend_from_slice(&input2.to_big_endian());
    data.extend_from_slice(&output1.to_big_endian());
    data.extend_from_slice(&output2.to_big_endian());

    Log {
        address: BOR_FEE_CONTRACT,
        topics,
        data: Bytes::from(data),
    }
}

/// Left-pads a 20-byte address to a 32-byte H256.
fn address_to_h256(addr: Address) -> H256 {
    let mut buf = [0u8; 32];
    buf[12..32].copy_from_slice(addr.as_bytes());
    H256(buf)
}
