use ethrex_storage::Store;
use tracing::error;

use crate::utils::RpcErr;

// TODO: Maybe these constants should be some kind of config.
// How many transactions to take as a price sample from a block.
const TXS_SAMPLE_SIZE: usize = 3;
// How many blocks we'll go back to calculate the estimate.
const BLOCK_RANGE_LOWER_BOUND_DEC: u64 = 20;

// The following comment is taken from the implementation of gas_price and is still valid, the logic was just moved here.

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
pub fn estimate_gas_tip(storage: &Store) -> Result<Option<u64>, RpcErr> {
    let latest_block_number = storage.get_latest_block_number()?;
    let block_range_lower_bound = latest_block_number.saturating_sub(BLOCK_RANGE_LOWER_BOUND_DEC);
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
        let Some(block_body) = storage.get_block_body(block_num)? else {
            error!("Block body for block number {block_num} is missing but is below the latest known block!");
            return Err(RpcErr::Internal(
                "Error calculating gas price: missing data".to_string(),
            ));
        };

        let base_fee = storage
            .get_block_header(latest_block_number)
            .ok()
            .flatten()
            .and_then(|header| header.base_fee_per_gas);

        // Previously we took the gas_price, now we take the effective_gas_tip and add the base_fee in the RPC
        // call if needed.
        let mut gas_tip_samples = block_body
            .transactions
            .into_iter()
            .filter_map(|tx| tx.effective_gas_tip(base_fee))
            .collect::<Vec<u64>>();

        gas_tip_samples.sort();
        results.extend(gas_tip_samples.into_iter().take(TXS_SAMPLE_SIZE));
    }
    results.sort();

    match results.get(results.len() / 2) {
        None => Ok(None),
        Some(gas) => Ok(Some(*gas)),
    }
}

// Tests for the estimate_gas_tip function.
#[cfg(test)]
mod tests {
    use crate::eth::fee_calculator::estimate_gas_tip;
    use crate::eth::test_utils::{
        add_eip1559_tx_blocks, add_legacy_tx_blocks, add_mixed_tx_blocks, setup_store,
        BASE_PRICE_IN_WEI,
    };

    #[tokio::test]
    async fn test_for_legacy_txs() {
        let storage = setup_store().await;
        add_legacy_tx_blocks(&storage, 20, 10).await;
        let gas_tip = estimate_gas_tip(&storage).unwrap().unwrap();
        assert_eq!(gas_tip, BASE_PRICE_IN_WEI);
    }

    #[tokio::test]
    async fn test_for_eip1559_txs() {
        let storage = setup_store().await;
        add_eip1559_tx_blocks(&storage, 20, 10).await;
        let gas_tip = estimate_gas_tip(&storage).unwrap().unwrap();
        assert_eq!(gas_tip, BASE_PRICE_IN_WEI);
    }

    #[tokio::test]
    async fn test_for_mixed_txs() {
        let storage = setup_store().await;
        add_mixed_tx_blocks(&storage, 20, 10).await;
        let gas_tip = estimate_gas_tip(&storage).unwrap().unwrap();
        assert_eq!(gas_tip, BASE_PRICE_IN_WEI);
    }

    #[tokio::test]
    async fn test_for_empty_blocks() {
        let storage = setup_store().await;
        let gas_tip = estimate_gas_tip(&storage).unwrap();
        assert_eq!(gas_tip, None);
    }
}
