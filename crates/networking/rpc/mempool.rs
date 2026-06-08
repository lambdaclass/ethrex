use std::collections::HashMap;

use ethrex_common::{Address, types::TxKind};
use ethrex_crypto::NativeCrypto;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{rpc::RpcApiContext, types::transaction::RpcTransaction, utils::RpcErr};

/// Maps account sender to its transactions indexed by nonce
type MempoolContentEntry = HashMap<Address, HashMap<u64, RpcTransaction>>;

/// Full content of the mempool
/// Transactions are grouped by sender and indexed by nonce
#[derive(Serialize, Deserialize)]
pub struct MempoolContent {
    pub pending: MempoolContentEntry,
    pub queued: MempoolContentEntry,
}

#[derive(Serialize, Deserialize)]
struct MempoolStatus {
    pending: String,
    queued: String,
}

type MempoolContentByNonce = HashMap<u64, RpcTransaction>;

#[derive(Serialize, Deserialize)]
pub struct MempoolContentFrom {
    pub pending: MempoolContentByNonce,
    pub queued: MempoolContentByNonce,
}

type MempoolInspectEntry = HashMap<Address, HashMap<u64, String>>;

#[derive(Serialize, Deserialize)]
pub struct MempoolInspect {
    pub pending: MempoolInspectEntry,
    pub queued: MempoolInspectEntry,
}

/// Handling of rpc endpoint `mempool_content`
pub fn content(context: RpcApiContext) -> Result<Value, RpcErr> {
    let transactions = context.blockchain.mempool.content()?;
    // Group transactions by sender and nonce and map them to rpc transactions
    let mut mempool_content = MempoolContentEntry::new();
    for tx in transactions {
        let sender_entry = mempool_content
            .entry(tx.sender(&NativeCrypto)?)
            .or_default();
        sender_entry.insert(tx.nonce(), RpcTransaction::build(tx, None, None, None)?);
    }
    let response = MempoolContent {
        pending: mempool_content,
        // We have no concept of "queued" transactions yet so we will leave this empty
        queued: MempoolContentEntry::new(),
    };
    Ok(serde_json::to_value(response)?)
}

pub fn status(context: RpcApiContext) -> Result<Value, RpcErr> {
    let pending = context.blockchain.mempool.status()?;
    // We have no concept of "queued" transactions yet so we will leave this as 0
    let queued = 0;

    let response = MempoolStatus {
        pending: format!("{pending:#x}"),
        queued: format!("{queued:#x}"),
    };

    Ok(serde_json::to_value(response)?)
}

/// Handling of rpc endpoint `txpool_contentFrom`
pub fn content_from(params: &Option<Vec<Value>>, context: RpcApiContext) -> Result<Value, RpcErr> {
    let params = params
        .as_ref()
        .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
    if params.len() != 1 {
        return Err(RpcErr::BadParams(format!(
            "Expected one param and {} were provided",
            params.len()
        )));
    }
    let address: Address = serde_json::from_value(params[0].clone())?;
    let transactions = context.blockchain.mempool.content()?;
    let mut by_nonce: MempoolContentByNonce = HashMap::new();
    for tx in transactions {
        if tx.sender(&NativeCrypto)? == address {
            by_nonce.insert(tx.nonce(), RpcTransaction::build(tx, None, None, None)?);
        }
    }
    let response = MempoolContentFrom {
        pending: by_nonce,
        // We have no concept of "queued" transactions yet so we will leave this empty
        queued: HashMap::new(),
    };
    Ok(serde_json::to_value(response)?)
}

/// Handling of rpc endpoint `txpool_inspect`
pub fn inspect(context: RpcApiContext) -> Result<Value, RpcErr> {
    let transactions = context.blockchain.mempool.content()?;
    let mut pending: MempoolInspectEntry = HashMap::new();
    for tx in transactions {
        let sender = tx.sender(&NativeCrypto)?;
        let gas_price = tx.gas_price();
        let summary = match tx.to() {
            TxKind::Call(to) => format!(
                "{to:#x}: {} wei + {} gas × {} wei",
                tx.value(),
                tx.gas_limit(),
                gas_price
            ),
            TxKind::Create => format!(
                "contract creation: {} wei + {} gas × {} wei",
                tx.value(),
                tx.gas_limit(),
                gas_price
            ),
        };
        pending
            .entry(sender)
            .or_default()
            .insert(tx.nonce(), summary);
    }
    let response = MempoolInspect {
        pending,
        // We have no concept of "queued" transactions yet so we will leave this empty
        queued: HashMap::new(),
    };
    Ok(serde_json::to_value(response)?)
}
