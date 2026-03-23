//! Bridge transaction analysis — extract needed accounts and storage slots.
//!
//! Follows the same pattern as zk-dex: iterate transactions, recover senders,
//! collect recipients and system accounts only when actually accessed.

use ethrex_common::types::{Block, TxKind, TxType};
use ethrex_common::{Address, H256};

use crate::common::handlers::constants::{
    BURN_ADDRESS, COMMON_BRIDGE_L2_ADDRESS, L2_TO_L1_MESSENGER_ADDRESS,
    MESSENGER_LAST_MESSAGE_ID_SLOT,
};

#[cfg(feature = "l2")]
use crate::common::app_execution::{FEE_TOKEN_RATIO_ADDRESS, FEE_TOKEN_REGISTRY_ADDRESS};
#[cfg(feature = "l2")]
use ethrex_common::types::block_execution_witness::ExecutionWitness;
#[cfg(feature = "l2")]
use ethrex_common::types::l2::fee_config::FeeConfig;

/// Analyze bridge transactions and collect needed accounts/storage.
///
/// Only includes accounts that are actually accessed during execution:
/// - Coinbase (always, for gas fees)
/// - TX senders (nonce/balance)
/// - TX recipients
/// - System contracts ONLY when called (withdrawal → BURN + MESSENGER)
/// - Fee vaults ONLY when configured
#[cfg(feature = "l2")]
pub fn analyze_bridge_transactions(
    blocks: &[Block],
    fee_configs: &[FeeConfig],
    _execution_witness: &ExecutionWitness,
) -> Result<(Vec<Address>, Vec<(Address, H256)>), String> {
    use std::collections::BTreeSet;

    let mut accounts: BTreeSet<Address> = BTreeSet::new();
    let mut storage_slots: BTreeSet<(Address, H256)> = BTreeSet::new();
    let mut has_withdrawal = false;

    // Block coinbases (always needed for gas fee distribution)
    for block in blocks {
        accounts.insert(block.header.coinbase);
    }

    // Process each transaction
    for block in blocks {
        for tx in &block.body.transactions {
            if tx.tx_type() == TxType::Privileged {
                // Deposit: extract recipient from calldata
                let data = tx.data();
                if data.len() >= 36 {
                    let recipient = Address::from_slice(&data[16..36]);
                    accounts.insert(recipient);
                }
                if let TxKind::Call(to) = tx.to() {
                    accounts.insert(to);
                }
                continue;
            }

            // Non-privileged: recover sender (required for nonce/balance)
            let sender = tx
                .sender()
                .map_err(|e| format!("sender recovery failed: {e}"))?;
            accounts.insert(sender);

            let to_addr = match tx.to() {
                TxKind::Call(addr) => addr,
                TxKind::Create => continue,
            };

            // Withdrawal to CommonBridgeL2
            if to_addr == COMMON_BRIDGE_L2_ADDRESS {
                has_withdrawal = true;
                accounts.insert(COMMON_BRIDGE_L2_ADDRESS);
                continue;
            }

            // System contract calls (only add if actually called)
            if to_addr == L2_TO_L1_MESSENGER_ADDRESS
                || to_addr == FEE_TOKEN_REGISTRY_ADDRESS
                || to_addr == FEE_TOKEN_RATIO_ADDRESS
            {
                accounts.insert(to_addr);
                continue;
            }

            // Regular ETH transfer destination
            accounts.insert(to_addr);
        }
    }

    // Withdrawal-required accounts/storage
    if has_withdrawal {
        accounts.insert(BURN_ADDRESS);
        accounts.insert(L2_TO_L1_MESSENGER_ADDRESS);
        storage_slots.insert((L2_TO_L1_MESSENGER_ADDRESS, MESSENGER_LAST_MESSAGE_ID_SLOT));
    }

    // Fee config vaults (only when configured)
    for fc in fee_configs {
        if let Some(vault) = fc.base_fee_vault {
            accounts.insert(vault);
        }
        if let Some(ref op) = fc.operator_fee_config {
            accounts.insert(op.operator_fee_vault);
        }
    }

    Ok((
        accounts.into_iter().collect(),
        storage_slots.into_iter().collect(),
    ))
}
