use ethrex_blockchain::constants::MAX_BLOB_GAS_PER_BLOCK;
use ethrex_core::types::{Block, Transaction};
use serde::Serialize;
use serde_json::Value;
use tracing::info;

use crate::{types::block_identifier::BlockIdentifier, utils::RpcErr, RpcApiContext, RpcHandler};
use ethrex_core::types::calculate_base_fee_per_blob_gas;
use ethrex_storage::Store;

#[derive(Clone, Debug)]
pub struct FeeHistoryRequest {
    pub block_count: u64,
    pub newest_block: BlockIdentifier,
    pub reward_percentiles: Option<Vec<f32>>,
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

impl RpcHandler for FeeHistoryRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<FeeHistoryRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() < 2 || params.len() > 3 {
            return Err(RpcErr::BadParams(format!(
                "Expected 2 or 3 params, got {}",
                params.len()
            )));
        };

        let reward_percentiles = match params.get(2).cloned() {
            Some(rp) => {
                let rp: Vec<f32> = serde_json::from_value(rp)?;
                let all_ok = rp
                    .windows(2)
                    .all(|w| w[0] <= w[1] || w[0] >= 0.0 && w[0] <= 100.0);
                // We want to return None if any value is wrong
                Some(
                    all_ok
                        .then_some(rp)
                        .ok_or(RpcErr::BadParams("Some of the params are wrong".to_owned()))?,
                )
            }
            None => None,
        };

        let block_count_str: String = serde_json::from_value(params[0].clone())?;
        let block_count_str = block_count_str.strip_prefix("0x").ok_or(RpcErr::BadParams(
            "Expected param to be 0x prefixed".to_owned(),
        ))?;

        Ok(FeeHistoryRequest {
            block_count: u64::from_str_radix(block_count_str, 16)
                .map_err(|error| RpcErr::BadParams(error.to_string()))?,
            newest_block: BlockIdentifier::parse(params[0].clone(), 0)?,
            reward_percentiles,
        })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        info!(
            "Requested fee history for {} blocks starting from {}",
            self.block_count, self.newest_block
        );

        if self.block_count == 0 {
            return serde_json::to_value(FeeHistoryResponse::default())
                .map_err(|error| RpcErr::Internal(error.to_string()));
        }

        let (start_block, end_block) =
            Self::get_range(storage, self.block_count, &self.newest_block)?;
        let oldest_block = start_block;
        let block_count = (end_block - start_block) as usize;
        let mut base_fee_per_gas = Vec::<u64>::with_capacity(block_count + 1);
        let mut base_fee_per_blob_gas = Vec::<u64>::with_capacity(block_count + 1);
        let mut gas_used_ratio = Vec::<f64>::with_capacity(block_count);
        let mut blob_gas_used_ratio = Vec::<f64>::with_capacity(block_count);
        let mut reward = Vec::<Vec<u64>>::with_capacity(block_count);

        for block_number in start_block..end_block {
            let header = storage
                .get_block_header(block_number)?
                .ok_or(RpcErr::Internal(format!(
                    "Could not get header for block {block_number}"
                )))?;
            let body = storage
                .get_block_body(block_number)?
                .ok_or(RpcErr::Internal(format!(
                    "Could not get body for block {block_number}"
                )))?;
            let blob_base_fee =
                calculate_base_fee_per_blob_gas(header.excess_blob_gas.unwrap_or_default());

            base_fee_per_gas.push(header.base_fee_per_gas.unwrap_or_default());
            base_fee_per_blob_gas.push(blob_base_fee);
            gas_used_ratio.push(header.gas_used as f64 / header.gas_limit as f64);
            blob_gas_used_ratio.push(
                header.blob_gas_used.unwrap_or_default() as f64 / MAX_BLOB_GAS_PER_BLOCK as f64,
            );

            if let Some(percentiles) = &self.reward_percentiles {
                let block = Block::new(header, body);
                reward.push(Self::calculate_percentiles_for_block(block, percentiles));
            }
        }

        // Now we project base_fee_per_gas and base_fee_per_blob_gas from last block
        let header = storage
            .get_block_header(end_block)?
            .ok_or(RpcErr::Internal(format!(
                "Could not get header for block {end_block}"
            )))?;

        let blob_base_fee =
            calculate_base_fee_per_blob_gas(header.excess_blob_gas.unwrap_or_default());
        base_fee_per_gas.push(header.base_fee_per_gas.unwrap_or_default());
        base_fee_per_blob_gas.push(blob_base_fee);

        let u64_to_hex_str = |x: u64| format!("0x{:x}", x);
        let response = FeeHistoryResponse {
            oldest_block: u64_to_hex_str(oldest_block),
            base_fee_per_gas: base_fee_per_gas.into_iter().map(u64_to_hex_str).collect(),
            base_fee_per_blob_gas: base_fee_per_blob_gas
                .into_iter()
                .map(u64_to_hex_str)
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

impl FeeHistoryRequest {
    fn get_range(
        storage: &Store,
        block_num: u64,
        finish_block: &BlockIdentifier,
    ) -> Result<(u64, u64), RpcErr> {
        // TODO: We should probably restrict how many blocks we are fetching to a certain limit

        // Get earliest block
        let earliest_block_num = storage
            .get_earliest_block_number()?
            .ok_or(RpcErr::Internal(
                "Could not get earliest block number".to_owned(),
            ))?;

        // Get latest block
        let latest_block_num = storage.get_latest_block_number()?.ok_or(RpcErr::Internal(
            "Could not get latest block number".to_owned(),
        ))?;

        // Get finish_block number
        let finish_block = finish_block
            .resolve_block_number(storage)?
            .ok_or(RpcErr::Internal(
                "Could not resolve block number".to_owned(),
            ))?;

        // finish block has to be included in the range
        let finish_block = finish_block + 1;

        // Acomodate finish_block to be <= latest_block
        let finish_block = finish_block.min(latest_block_num);

        // Acomodate start_block to be >= earliest_block
        let start_block = earliest_block_num.max(finish_block.saturating_sub(block_num));

        Ok((start_block, finish_block))
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
                Transaction::PrivilegedL2Transaction(t) => t
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
}
