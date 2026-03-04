//! Gas fee distribution handler.
//!
//! Replicates the L2 hook's gas fee distribution logic so that the guest
//! program's state transition matches the EVM exactly:
//!   1. Debit `effective_gas_price * gas_used` from sender
//!   2. Credit `priority_fee * gas_used` to coinbase
//!   3. Credit `base_fee * gas_used` to base_fee_vault (if configured)
//!   4. Credit `operator_fee * gas_used` to operator_fee_vault (if configured)

use ethrex_common::types::l2::fee_config::FeeConfig;
use ethrex_common::types::{BlockHeader, Transaction};
use ethrex_common::{Address, U256};

use crate::common::app_execution::AppCircuitError;
use crate::common::app_state::AppState;

/// Apply gas fee distribution matching the L2 EVM execution.
///
/// This replaces the previous `apply_gas_deduction` which only charged
/// `base_fee * gas_used` from the sender. The new implementation distributes
/// fees to coinbase and configured vaults exactly as the L2 hook does.
pub fn apply_gas_fee_distribution(
    state: &mut AppState,
    sender: Address,
    tx: &Transaction,
    gas_used: u64,
    block_header: &BlockHeader,
    fee_config: &FeeConfig,
) -> Result<(), AppCircuitError> {
    let base_fee = block_header.base_fee_per_gas.unwrap_or(0);
    let effective_price = tx
        .effective_gas_price(Some(base_fee))
        .unwrap_or(U256::from(base_fee));
    let gas = U256::from(gas_used);

    // 1. sender -= effective_price * gas_used
    let total_fee = effective_price.saturating_mul(gas);
    if !total_fee.is_zero() {
        state.debit_balance(sender, total_fee)?;
    }

    // 2. Compute operator_fee_per_gas (if configured)
    let op_fee_per_gas = fee_config
        .operator_fee_config
        .map(|c| U256::from(c.operator_fee_per_gas))
        .unwrap_or_default();

    // 3. priority_fee = effective_price - base_fee - operator_fee_per_gas
    let priority_fee = effective_price
        .saturating_sub(U256::from(base_fee))
        .saturating_sub(op_fee_per_gas);

    // 4. coinbase += priority_fee * gas_used
    let coinbase_credit = gas.saturating_mul(priority_fee);
    if !coinbase_credit.is_zero() {
        state.credit_balance(block_header.coinbase, coinbase_credit)?;
    }

    // 5. base_fee_vault += base_fee * gas_used (if configured)
    if let Some(vault) = fee_config.base_fee_vault {
        let credit = gas.saturating_mul(U256::from(base_fee));
        if !credit.is_zero() {
            state.credit_balance(vault, credit)?;
        }
    }

    // 6. operator_vault += operator_fee * gas_used (if configured)
    if let Some(op) = fee_config.operator_fee_config {
        let credit = gas.saturating_mul(U256::from(op.operator_fee_per_gas));
        if !credit.is_zero() {
            state.credit_balance(op.operator_fee_vault, credit)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::types::TxKind;
    use ethrex_common::types::l2::fee_config::OperatorFeeConfig;
    use ethrex_common::types::transaction::EIP1559Transaction;
    use ethrex_common::{H160, H256};

    use crate::common::app_state::AppState;
    use crate::common::app_types::AccountProof;
    use crate::common::handlers::constants::COMMON_BRIDGE_L2_ADDRESS;

    fn make_state(accounts: Vec<(Address, U256)>) -> AppState {
        let proofs: Vec<AccountProof> = accounts
            .iter()
            .map(|(addr, bal)| AccountProof {
                address: *addr,
                nonce: 0,
                balance: *bal,
                storage_root: H256::zero(),
                code_hash: H256::zero(),
                proof: vec![],
            })
            .collect();
        AppState::from_proofs(H256::zero(), proofs, vec![])
    }

    fn make_block_header(base_fee: u64, coinbase: Address) -> BlockHeader {
        BlockHeader {
            base_fee_per_gas: Some(base_fee),
            coinbase,
            ..Default::default()
        }
    }

    /// EIP-1559 TX: max_fee=10 gwei, max_priority=2 gwei, base_fee=7 gwei
    /// effective_gas_price = min(10, 7+2) = 9 gwei
    /// priority_fee = 9 - 7 = 2 gwei
    fn make_eip1559_tx(sender: Address) -> Transaction {
        let tx = EIP1559Transaction {
            chain_id: 1,
            nonce: 0,
            max_priority_fee_per_gas: 2_000_000_000, // 2 gwei
            max_fee_per_gas: 10_000_000_000,         // 10 gwei
            gas_limit: 200_000,
            to: TxKind::Call(COMMON_BRIDGE_L2_ADDRESS),
            value: U256::zero(),
            data: Bytes::new(),
            ..Default::default()
        };
        let _ = tx.sender_cache.set(sender);
        Transaction::EIP1559Transaction(tx)
    }

    #[test]
    fn gas_fee_debits_effective_price_not_just_base_fee() {
        let sender = H160([0xAA; 20]);
        let coinbase = H160([0xCB; 20]);
        let initial_balance = U256::from(1_000_000_000_000_000_000u64); // 1 ETH

        let mut state = make_state(vec![(sender, initial_balance), (coinbase, U256::zero())]);

        let tx = make_eip1559_tx(sender);
        let base_fee: u64 = 7_000_000_000; // 7 gwei
        let header = make_block_header(base_fee, coinbase);
        let gas_used: u64 = 100_000;

        apply_gas_fee_distribution(
            &mut state,
            sender,
            &tx,
            gas_used,
            &header,
            &FeeConfig::default(),
        )
        .unwrap();

        // effective_gas_price = min(10 gwei, 7 gwei + 2 gwei) = 9 gwei
        // total_fee = 9 gwei * 100_000 = 900_000 gwei = 0.0009 ETH
        let effective_price = U256::from(9_000_000_000u64);
        let expected_debit = effective_price * U256::from(gas_used);
        assert_eq!(
            state.get_balance(sender).unwrap(),
            initial_balance - expected_debit,
            "Sender should be debited effective_gas_price * gas_used"
        );
    }

    #[test]
    fn coinbase_receives_priority_fee() {
        let sender = H160([0xAA; 20]);
        let coinbase = H160([0xCB; 20]);
        let initial_balance = U256::from(1_000_000_000_000_000_000u64);

        let mut state = make_state(vec![(sender, initial_balance), (coinbase, U256::zero())]);

        let tx = make_eip1559_tx(sender);
        let base_fee: u64 = 7_000_000_000;
        let header = make_block_header(base_fee, coinbase);
        let gas_used: u64 = 100_000;

        apply_gas_fee_distribution(
            &mut state,
            sender,
            &tx,
            gas_used,
            &header,
            &FeeConfig::default(),
        )
        .unwrap();

        // priority_fee = effective(9 gwei) - base_fee(7 gwei) = 2 gwei
        // coinbase_credit = 2 gwei * 100_000 = 200_000 gwei
        let expected_coinbase = U256::from(2_000_000_000u64) * U256::from(gas_used);
        assert_eq!(
            state.get_balance(coinbase).unwrap(),
            expected_coinbase,
            "Coinbase should receive priority_fee * gas_used"
        );
    }

    #[test]
    fn base_fee_vault_receives_base_fee() {
        let sender = H160([0xAA; 20]);
        let coinbase = H160([0xCB; 20]);
        let vault = H160([0xDD; 20]);
        let initial_balance = U256::from(1_000_000_000_000_000_000u64);

        let mut state = make_state(vec![
            (sender, initial_balance),
            (coinbase, U256::zero()),
            (vault, U256::zero()),
        ]);

        let tx = make_eip1559_tx(sender);
        let base_fee: u64 = 7_000_000_000;
        let header = make_block_header(base_fee, coinbase);
        let gas_used: u64 = 100_000;
        let fee_config = FeeConfig {
            base_fee_vault: Some(vault),
            ..Default::default()
        };

        apply_gas_fee_distribution(&mut state, sender, &tx, gas_used, &header, &fee_config)
            .unwrap();

        // base_fee_vault gets base_fee * gas_used = 7 gwei * 100_000
        let expected_vault = U256::from(base_fee) * U256::from(gas_used);
        assert_eq!(
            state.get_balance(vault).unwrap(),
            expected_vault,
            "Base fee vault should receive base_fee * gas_used"
        );
    }

    #[test]
    fn operator_vault_receives_operator_fee() {
        let sender = H160([0xAA; 20]);
        let coinbase = H160([0xCB; 20]);
        let op_vault = H160([0xEE; 20]);
        let initial_balance = U256::from(1_000_000_000_000_000_000u64);

        let mut state = make_state(vec![
            (sender, initial_balance),
            (coinbase, U256::zero()),
            (op_vault, U256::zero()),
        ]);

        let tx = make_eip1559_tx(sender);
        let base_fee: u64 = 5_000_000_000; // 5 gwei
        let header = make_block_header(base_fee, coinbase);
        let gas_used: u64 = 100_000;
        let operator_fee_per_gas: u64 = 1_000_000_000; // 1 gwei

        let fee_config = FeeConfig {
            base_fee_vault: None,
            operator_fee_config: Some(OperatorFeeConfig {
                operator_fee_vault: op_vault,
                operator_fee_per_gas,
            }),
            l1_fee_config: None,
        };

        apply_gas_fee_distribution(&mut state, sender, &tx, gas_used, &header, &fee_config)
            .unwrap();

        // effective_gas_price = min(10, 5+2) = 7 gwei
        // operator_fee = 1 gwei * 100_000
        let expected_op = U256::from(operator_fee_per_gas) * U256::from(gas_used);
        assert_eq!(
            state.get_balance(op_vault).unwrap(),
            expected_op,
            "Operator vault should receive operator_fee_per_gas * gas_used"
        );

        // priority_fee = effective(7) - base(5) - operator(1) = 1 gwei
        let expected_coinbase = U256::from(1_000_000_000u64) * U256::from(gas_used);
        assert_eq!(
            state.get_balance(coinbase).unwrap(),
            expected_coinbase,
            "Coinbase should receive (priority - operator) * gas_used"
        );
    }

    #[test]
    fn fee_distribution_sums_match_total_debit() {
        //! Conservation law: sender debit == coinbase + base_vault + operator_vault
        let sender = H160([0xAA; 20]);
        let coinbase = H160([0xCB; 20]);
        let base_vault = H160([0xDD; 20]);
        let op_vault = H160([0xEE; 20]);
        let initial_balance = U256::from(10) * U256::from(1_000_000_000_000_000_000u64); // 10 ETH

        let mut state = make_state(vec![
            (sender, initial_balance),
            (coinbase, U256::zero()),
            (base_vault, U256::zero()),
            (op_vault, U256::zero()),
        ]);

        let tx = make_eip1559_tx(sender);
        let base_fee: u64 = 5_000_000_000; // 5 gwei
        let header = make_block_header(base_fee, coinbase);
        let gas_used: u64 = 100_000;
        let operator_fee_per_gas: u64 = 1_000_000_000; // 1 gwei

        let fee_config = FeeConfig {
            base_fee_vault: Some(base_vault),
            operator_fee_config: Some(OperatorFeeConfig {
                operator_fee_vault: op_vault,
                operator_fee_per_gas,
            }),
            l1_fee_config: None,
        };

        apply_gas_fee_distribution(&mut state, sender, &tx, gas_used, &header, &fee_config)
            .unwrap();

        let sender_debit = initial_balance - state.get_balance(sender).unwrap();
        let coinbase_credit = state.get_balance(coinbase).unwrap();
        let base_vault_credit = state.get_balance(base_vault).unwrap();
        let op_vault_credit = state.get_balance(op_vault).unwrap();

        assert_eq!(
            sender_debit,
            coinbase_credit + base_vault_credit + op_vault_credit,
            "Total fee debited from sender must equal sum of all credits.\n\
             sender_debit={sender_debit}, coinbase={coinbase_credit}, \
             base_vault={base_vault_credit}, op_vault={op_vault_credit}"
        );
    }
}
