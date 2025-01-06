use ethrex_blockchain::constants::MIN_GAS_LIMIT;
use tracing::error;

use crate::utils::RpcErr;
use crate::{RpcApiContext, RpcHandler};
use serde_json::Value;

// TODO: This does not need a struct,
// but I'm leaving it like this for consistency
// with the other RPC endpoints.
// The handle function could simply be
// a function called 'estimate'.
#[derive(Debug, Clone)]
pub struct MaxPriorityFee;

// TODO: Maybe these constants should be some kind of config.
// How many transactions to take as a price sample from a block.
const TXS_SAMPLE_SIZE: usize = 3;
// How many blocks we'll go back to calculate the estimate.
const BLOCK_RANGE_LOWER_BOUND_DEC: u64 = 20;

impl RpcHandler for MaxPriorityFee {
    fn parse(_: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(MaxPriorityFee {})
    }

    // Disclaimer:
    // This estimation is somewhat based on how currently go-ethereum does it.
    // Reference: https://github.com/ethereum/go-ethereum/blob/368e16f39d6c7e5cce72a92ec289adbfbaed4854/eth/gasprice/gasprice.go#L153
    // Although it will (probably) not yield the same result.
    // The idea here is to:
    // - Take the last 20 blocks (100% arbitrary, this could be more or less blocks)
    // - For each block, take the 3 txs with the lowest gas price (100% arbitrary)
    // - Join every fetched tx into a single vec and sort it.
    // - Return the one in the middle (what is also known as the 'median sample')
    // The intuition here is that we're sampling already accepted transactions,
    // fetched from recent blocks, so they should be real, representative values.
    // This specific implementation probably is not the best way to do this
    // but it works for now for a simple estimation, in the future
    // we can look into more sophisticated estimation methods, if needed.
    /// Estimate Gas Price based on already accepted transactions,
    /// as per the spec, this will be returned in wei.
    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let latest_block_number = context.storage.get_latest_block_number()?;
        let block_range_lower_bound =
            latest_block_number.saturating_sub(BLOCK_RANGE_LOWER_BOUND_DEC);
        // These are the blocks we'll use to estimate the price.
        let block_range = block_range_lower_bound..=latest_block_number;
        if block_range.is_empty() {
            error!(
                "Calculated block range from block {} \
                    up to block {} for gas price estimation is empty",
                block_range_lower_bound, latest_block_number
            );
            return Err(RpcErr::Internal("Error calculating gas price".to_string()));
        }
        let mut results = vec![];
        // TODO: Estimating gas price involves querying multiple blocks
        // and doing some calculations with each of them, let's consider
        // caching this result, also we can have a specific DB method
        // that returns a block range to not query them one-by-one.
        for block_num in block_range {
            let Some(block_body) = context.storage.get_block_body(block_num)? else {
                error!("Block body for block number {block_num} is missing but is below the latest known block!");
                return Err(RpcErr::Internal(
                    "Error calculating gas price: missing data".to_string(),
                ));
            };
            let mut max_priority_fee_samples = block_body
                .transactions
                .into_iter()
                .filter_map(|tx| tx.max_priority_fee())
                .collect::<Vec<u64>>();
            max_priority_fee_samples.sort();
            results.extend(max_priority_fee_samples.into_iter().take(TXS_SAMPLE_SIZE));
        }
        results.sort();

        let sample_gas = match results.get(results.len() / 2) {
            Some(gas) => *gas,
            None => {
                // If we don't have enough samples, we'll return the base fee or the min gas limit as a default.
                context
                    .storage
                    .get_block_header(latest_block_number)
                    .ok()
                    .flatten()
                    .and_then(|header| header.base_fee_per_gas)
                    .unwrap_or(MIN_GAS_LIMIT)
            }
        };

        let gas_as_hex = format!("0x{:x}", sample_gas);
        Ok(serde_json::Value::String(gas_as_hex))
    }
}

#[cfg(test)]
mod tests {
 }
