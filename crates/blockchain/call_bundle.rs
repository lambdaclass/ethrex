//! `eth_callBundle` execution, following reth's `EthBundle::call_bundle`.

use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{BlockHeader, ELASTICITY_MULTIPLIER, Transaction, TxKind, calculate_base_fee_per_gas},
};
use ethrex_crypto::{Crypto, NativeCrypto};
use ethrex_vm::{SimTxConfig, SimulationTxError, TxResult, TxValidationError};
use thiserror::Error;

use crate::{
    Blockchain,
    vm::{OverlaidVmDatabase, StoreVmDatabase},
};

const BUNDLE_BLOCK_TIME: u64 = 12;

#[derive(Clone, Debug, Default)]
pub struct CallBundleBlockEnv {
    pub number: u64,
    pub coinbase: Option<Address>,
    pub timestamp: Option<u64>,
    pub gas_limit: u64,
    pub difficulty: Option<U256>,
    pub base_fee: Option<u64>,
}

/// The state block's header with the bundle's overrides; the base fee defaults to the
/// next block's.
pub fn call_bundle_header(parent: &BlockHeader, env: &CallBundleBlockEnv) -> BlockHeader {
    let base_fee_per_gas = env.base_fee.or_else(|| {
        let parent_base_fee = parent.base_fee_per_gas?;
        calculate_base_fee_per_gas(
            parent.gas_limit,
            parent.gas_limit,
            parent.gas_used,
            parent_base_fee,
            ELASTICITY_MULTIPLIER,
        )
        .or(Some(parent_base_fee))
    });
    BlockHeader {
        parent_hash: parent.hash(),
        coinbase: env.coinbase.unwrap_or(parent.coinbase),
        state_root: parent.state_root,
        difficulty: env.difficulty.unwrap_or(parent.difficulty),
        number: env.number,
        gas_limit: env.gas_limit,
        timestamp: env
            .timestamp
            .unwrap_or_else(|| parent.timestamp.saturating_add(BUNDLE_BLOCK_TIME)),
        prev_randao: parent.prev_randao,
        base_fee_per_gas,
        blob_gas_used: parent.blob_gas_used.map(|_| 0),
        excess_blob_gas: parent.excess_blob_gas,
        parent_beacon_block_root: parent.parent_beacon_block_root,
        slot_number: parent.slot_number,
        ..Default::default()
    }
}

#[derive(Clone, Debug)]
pub struct CallBundleTxResult {
    pub tx_hash: H256,
    pub from: Address,
    pub to: Option<Address>,
    /// Post-refund gas, what the sender pays for.
    pub gas_used: u64,
    /// Effective priority fee per gas.
    pub gas_price: U256,
    pub gas_fees: U256,
    pub coinbase_diff: U256,
    pub eth_sent_to_coinbase: U256,
    pub success: bool,
    pub output: Bytes,
}

#[derive(Clone, Debug)]
pub struct CallBundleResult {
    pub bundle_hash: H256,
    pub bundle_gas_price: U256,
    pub coinbase_diff: U256,
    pub eth_sent_to_coinbase: U256,
    pub gas_fees: U256,
    pub results: Vec<CallBundleTxResult>,
    pub state_block_number: u64,
    pub total_gas_used: u64,
}

#[derive(Debug, Error)]
pub enum CallBundleError {
    #[error("max fee per gas less than block base fee")]
    FeeCapTooLow,
    #[error("invalid chain ID")]
    InvalidChainId,
    #[error("invalid transaction: {0}")]
    InvalidTx(TxValidationError),
    /// A transaction's gas limit exceeds what the earlier ones left of the block's.
    #[error("block gas limit reached: transaction gas limit {requested}, remaining {remaining}")]
    BlockGasLimitReached { requested: u64, remaining: u64 },
    #[error("internal error: {0}")]
    Internal(String),
}

fn internal(err: impl std::fmt::Display) -> CallBundleError {
    CallBundleError::Internal(err.to_string())
}

impl Blockchain {
    /// Runs `txs` in order in `header` on top of `parent`'s state, committing nothing.
    pub fn call_bundle(
        &self,
        parent: &BlockHeader,
        header: &BlockHeader,
        txs: &[(Transaction, Address)],
    ) -> Result<CallBundleResult, CallBundleError> {
        let chain_id = self.storage.get_chain_config().chain_id;
        let store_db =
            StoreVmDatabase::new(self.storage.clone(), parent.clone()).map_err(internal)?;
        // The bundle block sits right after the state block whatever `number` says, so
        // BLOCKHASH resolves up to and including the state block and is zero past it,
        // as it is for `eth_call` and `eth_simulateV1`. A bare `StoreVmDatabase` would
        // instead fail the request on a canonical block number that is not mined yet.
        let base_block_number = parent.number.saturating_add(1);
        let overlaid_db = OverlaidVmDatabase::new(store_db, Arc::default(), base_block_number);
        let mut evm = self.new_evm(overlaid_db).map_err(internal)?;
        let config = SimTxConfig {
            validate: true,
            trace_transfers: false,
        };
        let coinbase = header.coinbase;
        let base_fee = header.base_fee_per_gas.unwrap_or_default();

        let initial_coinbase_balance = evm.get_account_balance(coinbase).map_err(internal)?;
        let mut coinbase_balance = initial_coinbase_balance;
        let mut total_gas_used: u64 = 0;
        let mut total_gas_fees = U256::zero();
        let mut cumulative_gas_spent: u64 = 0;
        let mut hashes = Vec::with_capacity(txs.len() * 32);
        let mut results = Vec::with_capacity(txs.len());

        for (tx, sender) in txs {
            let tx_hash = tx.hash(&NativeCrypto);
            hashes.extend_from_slice(tx_hash.as_bytes());
            let gas_price = tx
                .effective_gas_tip(Some(base_fee))
                .ok_or(CallBundleError::FeeCapTooLow)?;
            if tx
                .chain_id()
                .is_some_and(|tx_chain_id| tx_chain_id != chain_id)
            {
                return Err(CallBundleError::InvalidChainId);
            }
            // The EVM checks each transaction's gas limit against the block's on its own;
            // the bundle as a whole has to fit the block too, which bounds a request's
            // work at one block's gas (`--rpc.gascap` by default).
            let remaining = header.gas_limit.saturating_sub(cumulative_gas_spent);
            if tx.gas_limit() > remaining {
                return Err(CallBundleError::BlockGasLimitReached {
                    requested: tx.gas_limit(),
                    remaining,
                });
            }

            let report = match evm.execute_tx_simulate(
                tx,
                header,
                &mut cumulative_gas_spent,
                *sender,
                &config,
            ) {
                Ok(report) => report,
                Err(SimulationTxError::Validation(error)) => {
                    return Err(CallBundleError::InvalidTx(error));
                }
                Err(SimulationTxError::Evm(error)) => return Err(internal(error)),
            };

            let gas_used = report.gas_spent;
            total_gas_used = total_gas_used.saturating_add(gas_used);
            let gas_fees = gas_price.saturating_mul(U256::from(gas_used));
            total_gas_fees = total_gas_fees.saturating_add(gas_fees);

            let balance_after = evm.get_account_balance(coinbase).map_err(internal)?;
            let coinbase_diff = balance_after.saturating_sub(coinbase_balance);
            coinbase_balance = balance_after;

            results.push(CallBundleTxResult {
                tx_hash,
                from: *sender,
                to: match tx.to() {
                    TxKind::Call(to) => Some(to),
                    TxKind::Create => None,
                },
                gas_used,
                gas_price,
                gas_fees,
                coinbase_diff,
                eth_sent_to_coinbase: coinbase_diff.saturating_sub(gas_fees),
                success: matches!(report.result, TxResult::Success),
                output: report.output,
            });
        }

        let coinbase_diff = coinbase_balance.saturating_sub(initial_coinbase_balance);
        let bundle_gas_price = coinbase_diff
            .checked_div(U256::from(total_gas_used))
            .unwrap_or_default();
        Ok(CallBundleResult {
            bundle_hash: H256(NativeCrypto.keccak256(&hashes)),
            bundle_gas_price,
            coinbase_diff,
            eth_sent_to_coinbase: coinbase_diff.saturating_sub(total_gas_fees),
            gas_fees: total_gas_fees,
            results,
            state_block_number: parent.number,
            total_gas_used,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parent() -> BlockHeader {
        BlockHeader {
            number: 10,
            timestamp: 1_000,
            coinbase: Address::repeat_byte(0xc0),
            gas_limit: 30_000_000,
            gas_used: 30_000_000,
            base_fee_per_gas: Some(1_000_000_000),
            difficulty: U256::from(7),
            prev_randao: H256::repeat_byte(0xaa),
            excess_blob_gas: Some(393_216),
            blob_gas_used: Some(262_144),
            state_root: H256::repeat_byte(0x55),
            ..Default::default()
        }
    }

    #[test]
    fn header_defaults_to_the_state_blocks_environment() {
        let parent = parent();
        let header = call_bundle_header(
            &parent,
            &CallBundleBlockEnv {
                number: 11,
                gas_limit: 50_000_000,
                ..Default::default()
            },
        );
        assert_eq!(header.parent_hash, parent.hash());
        assert_eq!(header.number, 11);
        assert_eq!(header.timestamp, 1_012);
        assert_eq!(header.coinbase, parent.coinbase);
        assert_eq!(header.gas_limit, 50_000_000);
        assert_eq!(header.difficulty, parent.difficulty);
        assert_eq!(header.prev_randao, parent.prev_randao);
        assert_eq!(header.excess_blob_gas, parent.excess_blob_gas);
        assert_eq!(header.blob_gas_used, Some(0));
        assert_eq!(header.state_root, parent.state_root);
        // A full parent raises the next base fee by 1/8.
        assert_eq!(header.base_fee_per_gas, Some(1_125_000_000));
    }

    #[test]
    fn header_applies_overrides() {
        let header = call_bundle_header(
            &parent(),
            &CallBundleBlockEnv {
                number: 42,
                coinbase: Some(Address::repeat_byte(0xcb)),
                timestamp: Some(5_000),
                gas_limit: 1_000_000,
                difficulty: Some(U256::from(9)),
                base_fee: Some(3),
            },
        );
        assert_eq!(header.number, 42);
        assert_eq!(header.coinbase, Address::repeat_byte(0xcb));
        assert_eq!(header.timestamp, 5_000);
        assert_eq!(header.gas_limit, 1_000_000);
        assert_eq!(header.difficulty, U256::from(9));
        assert_eq!(header.base_fee_per_gas, Some(3));
    }

    #[test]
    fn pre_london_state_block_has_no_base_fee_unless_given() {
        let parent = BlockHeader {
            base_fee_per_gas: None,
            ..parent()
        };
        let env = CallBundleBlockEnv {
            number: 11,
            gas_limit: 30_000_000,
            ..Default::default()
        };
        assert_eq!(call_bundle_header(&parent, &env).base_fee_per_gas, None);
        let env = CallBundleBlockEnv {
            base_fee: Some(5),
            ..env
        };
        assert_eq!(call_bundle_header(&parent, &env).base_fee_per_gas, Some(5));
    }
}
