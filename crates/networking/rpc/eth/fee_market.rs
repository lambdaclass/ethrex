use ethrex_blockchain::payload::calc_gas_limit;
use ethrex_common::{
    U256,
    constants::GAS_PER_BLOB,
    types::{
        Block, BlockHeader, ELASTICITY_MULTIPLIER, Fork, ForkBlobSchedule, Transaction,
        calc_excess_blob_gas, calculate_base_fee_per_blob_gas, calculate_base_fee_per_gas,
    },
};
use serde::Serialize;
use serde_json::Value;
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
    fn parse(params: &Option<Vec<Value>>) -> Result<FeeHistoryRequest, RpcErr> {
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
        let rp: Vec<f32> = serde_json::from_value(params[2].clone())?;
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
            newest_block: BlockIdentifier::parse(params[1].clone(), 0)?,
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
        // `get_range` clamps `start` up to the earliest available block but leaves
        // `end` where the caller put it, so an entirely-unavailable range (every
        // requested block below the prune cutoff / snap-sync pivot) comes back
        // inverted. Report it as empty, matching the `block_count == 0` case above;
        // subtracting here would underflow.
        if start_block > end_block {
            return serde_json::to_value(FeeHistoryResponse::default())
                .map_err(|error| RpcErr::Internal(error.to_string()));
        }
        let oldest_block = start_block;
        let block_count = (end_block - start_block + 1) as usize;
        let mut base_fee_per_gas = vec![0_u64; block_count + 1];
        let mut base_fee_per_blob_gas = vec![U256::zero(); block_count + 1];
        let mut gas_used_ratio = vec![0_f64; block_count];
        let mut blob_gas_used_ratio = vec![0_f64; block_count];
        let mut reward = Vec::<Vec<u64>>::with_capacity(block_count);

        for block_number in start_block..=end_block {
            let idx: usize = (block_number - start_block) as usize;
            let header = storage
                .get_block_header(block_number)?
                .ok_or(RpcErr::Internal(format!(
                    "Could not get header for block {block_number}"
                )))?;
            let body = storage
                .get_block_body(block_number)
                .await?
                .ok_or(RpcErr::Internal(format!(
                    "Could not get body for block {block_number}"
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
            if !self.reward_percentiles.is_empty() {
                reward.push(calculate_percentiles_for_block(
                    Block::new(header, body),
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
                .map(|v| v.into_iter().map(u64_to_hex_str).collect())
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
    let latest_block_num = storage.get_latest_block_number().await?;
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

fn calculate_percentiles_for_block(block: Block, percentiles: &[f32]) -> Vec<u64> {
    let base_fee_per_gas = block.header.base_fee_per_gas.unwrap_or_default();
    let mut effective_priority_fees: Vec<u64> = block
        .body
        .transactions
        .into_iter()
        .map(|t: Transaction| match t {
            Transaction::LegacyTransaction(_) | Transaction::EIP2930Transaction(_) => 0,
            Transaction::EIP1559Transaction(t) => t
                .max_priority_fee_per_gas
                .min(t.max_fee_per_gas.saturating_sub(base_fee_per_gas)),
            Transaction::EIP4844Transaction(t) => t
                .max_priority_fee_per_gas
                .min(t.max_fee_per_gas.saturating_sub(base_fee_per_gas)),
            Transaction::EIP7702Transaction(t) => t
                .max_priority_fee_per_gas
                .min(t.max_fee_per_gas.saturating_sub(base_fee_per_gas)),
            Transaction::PrivilegedL2Transaction(t) => t
                .max_priority_fee_per_gas
                .min(t.max_fee_per_gas.saturating_sub(base_fee_per_gas)),
            Transaction::FeeTokenTransaction(t) => t
                .max_priority_fee_per_gas
                .min(t.max_fee_per_gas.saturating_sub(base_fee_per_gas)),
            Transaction::FrameTransaction(t) => t
                .max_priority_fee_per_gas
                .min(t.max_fee_per_gas.saturating_sub(base_fee_per_gas)),
        })
        .collect();

    effective_priority_fees.sort();
    let t_len = effective_priority_fees.len() as f32;

    percentiles
        .iter()
        .map(|x: &f32| {
            let i = (x * t_len / 100_f32) as usize;
            effective_priority_fees.get(i).cloned().unwrap_or_default()
        })
        .collect()
}

#[cfg(test)]
mod pruned_range_tests {
    use super::*;
    use crate::test_utils::{add_legacy_tx_blocks, default_context_with_storage, setup_store};

    /// Regression: `eth_feeHistory` must not panic when the requested range lies
    /// entirely below the earliest available block.
    ///
    /// `get_range` clamps `start` up to `EarliestBlockNumber` but leaves `end` at the
    /// caller's `newestBlock`, so those two can cross. `handle` then computed
    /// `end - start + 1`, which underflows: in release builds (no `overflow-checks`)
    /// that wraps to ~1.8e19 and the subsequent `vec![0; block_count + 1]` aborts the
    /// connection task with a capacity-overflow panic.
    ///
    /// Reachable without `--history.retention`: snap-sync completion and the v3→v4
    /// migration both set `EarliestBlockNumber` to the pivot, and asking for fee
    /// history over pre-pivot blocks is an ordinary wallet/explorer query.
    #[tokio::test]
    async fn fee_history_below_earliest_block_returns_empty_not_panic() {
        let storage = setup_store().await;
        add_legacy_tx_blocks(&storage, 20, 1).await;
        // Simulate a snap-synced / pruned node whose history starts at 15.
        storage.advance_earliest_block_number(15).await.unwrap();

        let context = default_context_with_storage(storage).await;
        let request = FeeHistoryRequest {
            block_count: 1,
            // Well below `earliest`, so `get_range` returns an inverted pair.
            newest_block: BlockIdentifier::Number(3),
            reward_percentiles: vec![],
        };

        // The assertion that matters is that this returns at all rather than
        // panicking; the shape is the same empty response `block_count == 0` gives.
        let response = request.handle(context).await.unwrap();
        let expected = serde_json::to_value(FeeHistoryResponse::default()).unwrap();
        assert_eq!(response, expected);
    }

    /// A range that only *partially* precedes the earliest block still returns the
    /// available portion — the guard must not swallow satisfiable requests.
    #[tokio::test]
    async fn fee_history_straddling_earliest_block_returns_available_portion() {
        let storage = setup_store().await;
        add_legacy_tx_blocks(&storage, 20, 1).await;
        storage.advance_earliest_block_number(15).await.unwrap();

        let context = default_context_with_storage(storage).await;
        // Asks for 10 blocks ending at 18, i.e. 9..=18; clamped to 15..=18.
        let request = FeeHistoryRequest {
            block_count: 10,
            newest_block: BlockIdentifier::Number(18),
            reward_percentiles: vec![],
        };

        let response = request.handle(context).await.unwrap();
        assert_eq!(response["oldestBlock"], serde_json::json!("0xf"));
        // 4 blocks (15..=18) plus the projected next-block value.
        assert_eq!(response["baseFeePerGas"].as_array().unwrap().len(), 5);
        assert_eq!(response["gasUsedRatio"].as_array().unwrap().len(), 4);
    }
}
