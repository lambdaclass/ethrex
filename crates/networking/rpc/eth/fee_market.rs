use ethrex_blockchain::payload::calc_gas_limit;
use ethrex_common::{
    U256,
    constants::GAS_PER_BLOB,
    types::{
        Block, BlockHeader, ELASTICITY_MULTIPLIER, Fork, ForkBlobSchedule, Receipt, Transaction,
        calc_excess_blob_gas, calculate_base_fee_per_blob_gas, calculate_base_fee_per_gas,
    },
};
use serde::Serialize;
use serde_json::{Value, value::RawValue};
use tracing::debug;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    types::block_identifier::BlockIdentifier,
    utils::{RpcErr, parse_json_hex},
};
use ethrex_storage::Store;

// Those are some offspec constants
const MAX_PERCENTILE_ARRAY_LEN: usize = 128;
const MAX_BLOCK_COUNT: u64 = 1024;

#[derive(Clone, Debug)]
pub struct FeeHistoryRequest {
    pub block_count: u64,
    pub newest_block: BlockIdentifier,
    pub reward_percentiles: Vec<f32>,
}

#[derive(Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FeeHistoryResponse {
    pub oldest_block: String,
    pub base_fee_per_gas: Vec<String>,
    pub base_fee_per_blob_gas: Vec<String>,
    pub gas_used_ratio: Vec<f64>,
    pub blob_gas_used_ratio: Vec<f64>,
    pub reward: Vec<Vec<String>>,
}

// Implemented by reading:
// - https://github.com/ethereum/EIPs/blob/master/EIPS/eip-4844.md
// - https://ethereum.github.io/execution-apis/api-documentation/
// - https://github.com/ethereum/go-ethereum/blob/master/eth/gasprice/feehistory.go
impl RpcHandler for FeeHistoryRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<FeeHistoryRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 3 {
            return Err(RpcErr::BadParams(format!(
                "Expected 3 params, got {}",
                params.len()
            )));
        };
        let block_count: u64 = parse_json_hex(&params[0]).map_err(RpcErr::BadParams)?;
        // NOTE: This check is offspec
        if block_count > MAX_BLOCK_COUNT {
            return Err(RpcErr::BadParams(
                "Too large block_count parameter".to_owned(),
            ));
        }
        let rp: Vec<f32> = serde_json::from_str(params[2].get())?;
        // NOTE: This check is offspec
        if rp.len() > MAX_PERCENTILE_ARRAY_LEN {
            return Err(RpcErr::BadParams(format!(
                "Wrong size reward_percentiles parameter, must be {MAX_PERCENTILE_ARRAY_LEN} at max"
            )));
        }
        // Restric them to be monotnically increasing and in the range [0.0; 100.0]
        let mut ok = rp.iter().all(|a| *a >= 0.0 && *a <= 100.0);
        ok &= rp.windows(2).all(|w| w[0] <= w[1]);
        if !ok {
            return Err(RpcErr::BadParams(
                "Wrong reward_percentiles parameter".to_owned(),
            ));
        }

        Ok(FeeHistoryRequest {
            block_count,
            newest_block: BlockIdentifier::parse_param(&params[1], 0)?,
            reward_percentiles: rp,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        let config = storage.get_chain_config();
        debug!(
            "Requested fee history for {} blocks starting from {}",
            self.block_count, self.newest_block
        );

        if self.block_count == 0 {
            return serde_json::to_value(FeeHistoryResponse::default())
                .map_err(|error| RpcErr::Internal(error.to_string()));
        }

        let (start_block, end_block) =
            get_range(storage, self.block_count, &self.newest_block).await?;
        let oldest_block = start_block;
        let block_count = (end_block - start_block + 1) as usize;
        let mut base_fee_per_gas = vec![0_u64; block_count + 1];
        let mut base_fee_per_blob_gas = vec![U256::zero(); block_count + 1];
        let mut gas_used_ratio = vec![0_f64; block_count];
        let mut blob_gas_used_ratio = vec![0_f64; block_count];
        let mut reward = Vec::<Vec<U256>>::with_capacity(block_count);

        for block_number in start_block..=end_block {
            let idx: usize = (block_number - start_block) as usize;
            let header = storage
                .get_block_header(block_number)?
                .ok_or(RpcErr::Internal(format!(
                    "Could not get header for block {block_number}"
                )))?;

            let blob_schedule_opt = config.get_fork_blob_schedule(header.timestamp);
            let max_blob_gas_per_block =
                blob_schedule_opt.map(|schedule| schedule.max * GAS_PER_BLOB);
            let blob_gas_used_r = match (header.blob_gas_used, max_blob_gas_per_block) {
                (Some(blob_gas_used), Some(max_blob_gas)) => {
                    blob_gas_used as f64 / max_blob_gas as f64
                }
                _ => 0.0,
            };

            let blob_schedule = blob_schedule_opt.unwrap_or_default();

            let fork = config.get_fork(header.timestamp);

            let blob_base_fee = calculate_base_fee_per_blob_gas(
                header.excess_blob_gas.unwrap_or_default(),
                blob_schedule.base_fee_update_fraction,
            );

            base_fee_per_gas[idx] = header.base_fee_per_gas.unwrap_or_default();
            base_fee_per_blob_gas[idx] = blob_base_fee;
            gas_used_ratio[idx] = header.gas_used as f64 / header.gas_limit as f64;
            blob_gas_used_ratio[idx] = blob_gas_used_r;

            if block_number == end_block {
                (base_fee_per_gas[idx + 1], base_fee_per_blob_gas[idx + 1]) =
                    project_next_block_base_fee_values(
                        &header,
                        blob_schedule,
                        fork,
                        context.gas_ceil,
                    )?;
            }
            // Only the rewards need the block's transactions and receipts, so they
            // are read only when percentiles are requested.
            if !self.reward_percentiles.is_empty() {
                let body = storage
                    .get_block_body(block_number)
                    .await?
                    .ok_or(RpcErr::Internal(format!(
                        "Could not get body for block {block_number}"
                    )))?;
                let receipts = storage.get_receipts_for_block(&header.hash()).await?;
                reward.push(calculate_percentiles_for_block(
                    Block::new(header, body),
                    &receipts,
                    &self.reward_percentiles,
                ));
            }
        }

        let u64_to_hex_str = |x: u64| format!("0x{x:x}");
        let u256_to_hex_str = |x: U256| format!("{x:#x}");
        let response = FeeHistoryResponse {
            oldest_block: u64_to_hex_str(oldest_block),
            base_fee_per_gas: base_fee_per_gas.into_iter().map(u64_to_hex_str).collect(),
            base_fee_per_blob_gas: base_fee_per_blob_gas
                .into_iter()
                .map(u256_to_hex_str)
                .collect(),
            gas_used_ratio,
            blob_gas_used_ratio,
            reward: reward
                .into_iter()
                .map(|v| v.into_iter().map(u256_to_hex_str).collect())
                .collect(),
        };
        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

// Project base_fee_per_gas and base_fee_per_blob_gas of next block, from provided block
fn project_next_block_base_fee_values(
    header: &BlockHeader,
    schedule: ForkBlobSchedule,
    fork: Fork,
    gas_ceil: u64,
) -> Result<(u64, U256), RpcErr> {
    // NOTE: Given that this client supports the Paris fork and later versions, we are sure that the next block
    // will have the London update active, so the base fee calculation makes sense
    // Geth performs a validation for this case:
    // -> https://github.com/ethereum/go-ethereum/blob/master/eth/gasprice/feehistory.go#L93
    let next_gas_limit = calc_gas_limit(header.gas_limit, gas_ceil);
    let base_fee_per_gas = calculate_base_fee_per_gas(
        next_gas_limit,
        header.gas_limit,
        header.gas_used,
        header.base_fee_per_gas.unwrap_or_default(),
        ELASTICITY_MULTIPLIER,
    )
    .unwrap_or_default();
    let next_excess_blob_gas = calc_excess_blob_gas(header, schedule, fork);
    let base_fee_per_blob =
        calculate_base_fee_per_blob_gas(next_excess_blob_gas, schedule.base_fee_update_fraction);
    Ok((base_fee_per_gas, base_fee_per_blob))
}

async fn get_range(
    storage: &Store,
    block_count: u64,
    expected_finish_block: &BlockIdentifier,
) -> Result<(u64, u64), RpcErr> {
    // NOTE: The amount of blocks to retrieve is capped by MAX_BLOCK_COUNT

    // Get earliest block
    let earliest_block_num = storage.get_earliest_block_number().await?;
    // Get latest block
    let latest_block_num = storage.get_latest_block_number()?;
    // Get the expected finish block number from the parameter
    let expected_finish_block_num = expected_finish_block
        .resolve_block_number(storage)
        .await?
        .ok_or(RpcErr::Internal(
            "Could not resolve block number".to_owned(),
        ))?;
    // Calculate start and finish block numbers, considering finish block inclusion
    let finish_block_num = expected_finish_block_num.min(latest_block_num);
    let expected_start_block_num = (finish_block_num + 1).saturating_sub(block_count);
    let start_block_num = earliest_block_num.max(expected_start_block_num);

    Ok((start_block_num, finish_block_num))
}

/// Samples the block's effective priority fees per gas at the given percentiles, as
/// execution-apis specifies for `eth_feeHistory`: the transactions are sorted by
/// effective tip and each percentile picks the tip of the transaction at which the
/// cumulative gas used reaches that share of the block's gas, so the samples are
/// weighted by gas used. An empty block samples zeroes.
///
/// A block whose receipts are not stored (a snap pivot that was never executed)
/// weights each transaction by its gas limit instead.
fn calculate_percentiles_for_block(
    block: Block,
    receipts: &[Receipt],
    percentiles: &[f32],
) -> Vec<U256> {
    let base_fee_per_gas = block.header.base_fee_per_gas;
    let transactions = &block.body.transactions;
    let gas_used: Vec<u64> = if receipts.len() == transactions.len() {
        let mut previous_cumulative_gas = 0;
        receipts
            .iter()
            .map(|receipt| {
                let gas_used = receipt
                    .cumulative_gas_used
                    .saturating_sub(previous_cumulative_gas);
                previous_cumulative_gas = receipt.cumulative_gas_used;
                gas_used
            })
            .collect()
    } else {
        transactions.iter().map(Transaction::gas_limit).collect()
    };
    // (effective tip, gas used) per transaction. A legacy or EIP-2930 transaction
    // tips its gas price minus the base fee (EIP-1559).
    let mut tips_and_gas: Vec<(U256, u64)> = transactions
        .iter()
        .zip(gas_used)
        .map(|(tx, gas_used)| {
            (
                tx.effective_gas_tip(base_fee_per_gas).unwrap_or_default(),
                gas_used,
            )
        })
        .collect();
    let Some(last_index) = tips_and_gas.len().checked_sub(1) else {
        return vec![U256::zero(); percentiles.len()];
    };
    tips_and_gas.sort_by_key(|(tip, _)| *tip);
    let total_gas_used: u64 = tips_and_gas.iter().map(|(_, gas)| gas).sum();

    let mut index = 0;
    let mut sum_gas_used = tips_and_gas[0].1;
    percentiles
        .iter()
        .map(|percentile| {
            let threshold = (total_gas_used as f64 * f64::from(*percentile) / 100.0) as u64;
            while sum_gas_used < threshold && index < last_index {
                index += 1;
                sum_gas_used += tips_and_gas[index].1;
            }
            tips_and_gas[index].0
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::test_utils::{add_eip1559_tx_blocks, default_context_with_storage, setup_store};

    async fn fee_history(context: &RpcApiContext, reward_percentiles: Vec<f32>) -> Value {
        FeeHistoryRequest {
            block_count: 5,
            newest_block: BlockIdentifier::Number(5),
            reward_percentiles,
        }
        .handle(context.clone())
        .await
        .unwrap()
    }

    /// Requesting rewards adds the `reward` field and leaves every other field as
    /// it is without them, which are computed from the headers alone.
    #[tokio::test]
    async fn rewards_only_add_the_reward_field() {
        let storage = setup_store().await;
        add_eip1559_tx_blocks(&storage, 5, 10).await;
        let context = default_context_with_storage(storage).await;

        let without_rewards = fee_history(&context, vec![]).await;
        let mut with_rewards = fee_history(&context, vec![25.0, 75.0]).await;

        let rewards = with_rewards
            .as_object_mut()
            .unwrap()
            .remove("reward")
            .unwrap();
        assert_eq!(rewards.as_array().unwrap().len(), 5);
        assert!(
            rewards
                .as_array()
                .unwrap()
                .iter()
                .all(|block| block.as_array().unwrap().len() == 2)
        );
        let mut without_rewards = without_rewards;
        assert_eq!(
            without_rewards
                .as_object_mut()
                .unwrap()
                .remove("reward")
                .unwrap(),
            serde_json::json!([])
        );
        assert_eq!(without_rewards, with_rewards);
        assert_eq!(without_rewards["gasUsedRatio"].as_array().unwrap().len(), 5);
    }

    const GWEI: u64 = 1_000_000_000;

    fn legacy_tx(gas_price_gwei: u64) -> Transaction {
        Transaction::LegacyTransaction(ethrex_common::types::LegacyTransaction {
            gas_price: U256::from(gas_price_gwei * GWEI),
            gas: 1_000_000,
            ..Default::default()
        })
    }

    fn eip1559_tx(max_fee_gwei: u64, tip_gwei: u64) -> Transaction {
        Transaction::EIP1559Transaction(ethrex_common::types::EIP1559Transaction {
            max_fee_per_gas: max_fee_gwei * GWEI,
            max_priority_fee_per_gas: tip_gwei * GWEI,
            gas_limit: 1_000_000,
            ..Default::default()
        })
    }

    /// Stores block 1 (base fee 1 gwei) with `txs` and, unless `gas_used` is empty,
    /// receipts whose per-transaction gas used is `gas_used`, makes it the head and
    /// returns its `reward` entry.
    async fn block_rewards(
        txs: Vec<Transaction>,
        gas_used: &[u64],
        percentiles: Vec<f32>,
    ) -> Vec<String> {
        use ethrex_common::types::{BlockBody, Receipt};
        let storage = setup_store().await;
        let genesis = storage.get_block_header(0).unwrap().unwrap();
        let mut cumulative = 0;
        let receipts: Vec<Receipt> = txs
            .iter()
            .zip(gas_used)
            .map(|(tx, gas)| {
                cumulative += gas;
                Receipt::new(tx.tx_type(), true, cumulative, vec![])
            })
            .collect();
        let header = BlockHeader {
            parent_hash: genesis.hash(),
            number: 1,
            timestamp: genesis.timestamp + 12,
            gas_limit: 30_000_000,
            gas_used: cumulative,
            base_fee_per_gas: Some(GWEI),
            ..Default::default()
        };
        let block = Block::new(
            header,
            BlockBody {
                transactions: txs,
                ommers: vec![],
                withdrawals: None,
            },
        );
        let hash = block.hash();
        storage.add_block(block).await.unwrap();
        if !receipts.is_empty() {
            storage.add_receipts(hash, receipts).await.unwrap();
        }
        storage
            .forkchoice_update(vec![], 1, hash, None, None)
            .await
            .unwrap();
        let context = default_context_with_storage(storage).await;
        let context: &RpcApiContext = &context;
        let response = FeeHistoryRequest {
            block_count: 1,
            newest_block: BlockIdentifier::Number(1),
            reward_percentiles: percentiles,
        }
        .handle(context.clone())
        .await
        .unwrap();
        serde_json::from_value::<Vec<Vec<String>>>(response["reward"].clone())
            .unwrap()
            .remove(0)
    }

    /// The 100th percentile is the highest tip in the block, not zero.
    #[tokio::test]
    async fn hundredth_percentile_is_the_highest_tip() {
        let txs = vec![eip1559_tx(10, 1), eip1559_tx(10, 2), eip1559_tx(10, 3)];
        let rewards = block_rewards(txs, &[21_000, 21_000, 21_000], vec![0.0, 100.0]).await;
        assert_eq!(rewards, ["0x3b9aca00", "0xb2d05e00"]);
    }

    /// A legacy transaction tips its gas price minus the base fee (EIP-1559), so a
    /// block of 3 gwei legacy transactions over a 1 gwei base fee rewards 2 gwei.
    #[tokio::test]
    async fn legacy_transactions_reward_their_effective_tip() {
        let txs = vec![legacy_tx(3), legacy_tx(3)];
        let rewards = block_rewards(txs, &[21_000, 21_000], vec![50.0]).await;
        assert_eq!(rewards, ["0x77359400"]);
    }

    /// Percentiles are weighted by gas used: a transaction that used most of the
    /// block's gas sets the median tip, however few transactions sit around it.
    #[tokio::test]
    async fn rewards_are_weighted_by_gas_used() {
        let txs = vec![eip1559_tx(10, 1), eip1559_tx(10, 2), eip1559_tx(10, 3)];
        let rewards = block_rewards(txs, &[21_000, 21_000, 1_000_000], vec![50.0]).await;
        assert_eq!(rewards, ["0xb2d05e00"]);
    }

    /// Without stored receipts, each transaction is weighted by its gas limit.
    #[tokio::test]
    async fn rewards_fall_back_to_gas_limits_without_receipts() {
        let small = |tip| match eip1559_tx(10, tip) {
            Transaction::EIP1559Transaction(tx) => {
                Transaction::EIP1559Transaction(ethrex_common::types::EIP1559Transaction {
                    gas_limit: 21_000,
                    ..tx
                })
            }
            _ => unreachable!(),
        };
        let txs = vec![small(1), small(2), eip1559_tx(10, 3)];
        let rewards = block_rewards(txs, &[], vec![50.0]).await;
        assert_eq!(rewards, ["0xb2d05e00"]);
    }
}
