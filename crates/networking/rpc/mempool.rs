use std::collections::HashMap;

use ethrex_common::Address;
use serde::Serialize;
use serde_json::Value;

use crate::{rpc::RpcApiContext, types::transaction::RpcTransaction, utils::RpcErr};

/// Maps account sender to its transactions indexed by nonce
type MempoolContentEntry = HashMap<Address, HashMap<u64, RpcTransaction>>;

/// Full content of the mempool
/// Transactions are grouped by sender and indexed by nonce
#[derive(Serialize)]
struct MempoolContent {
    pending: MempoolContentEntry,
    queued: MempoolContentEntry,
}

/// Handling of rpc endpoint `mempool_content`
pub async fn content(context: RpcApiContext) -> Result<Value, RpcErr> {
    let latest_block_number = context.storage.get_latest_block_number().await?;
    let latest_block_hash = context
        .storage
        .get_canonical_block_hash(latest_block_number)
        .await?
        .ok_or(RpcErr::Internal("Missing latest block hash".to_string()))?;
    let transactions = context.blockchain.mempool.content()?;
    // Group transactions by sender and nonce and map them to rpc transactions
    let mut mempool_content = MempoolContentEntry::new();
    for tx in transactions {
        let sender_entry = mempool_content.entry(tx.sender()).or_default();
        sender_entry.insert(
            tx.nonce(),
            RpcTransaction::build(tx, latest_block_number, latest_block_hash, None),
        );
    }
    let response = MempoolContent {
        pending: mempool_content,
        // We have no concept of "queued" transactions yet so we will leave this empty
        queued: MempoolContentEntry::new(),
    };
    Ok(serde_json::to_value(response)?)
}
