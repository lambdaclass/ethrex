//! t8n output writing: `result.json`, the post-state `alloc.json`, and the
//! transactions RLP, in geth `evm t8n` formats.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ethrex_blockchain::payload::PayloadBuildResult;
use ethrex_common::types::{GenesisAccount, Log, Receipt, Transaction, bloom_from_logs};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::{Crypto, NativeCrypto};
use ethrex_rlp::encode::RLPEncode;
use serde_json::{Value, json};

use super::T8nArgs;
use super::error::T8nError;

/// A rejected transaction: its index in the input list and the reason.
pub struct Rejection {
    pub index: usize,
    pub error: String,
}

pub fn write_outputs(
    args: &T8nArgs,
    pre_alloc: &BTreeMap<Address, GenesisAccount>,
    result: &PayloadBuildResult,
    rejections: &[Rejection],
    block_exception: Option<String>,
) -> Result<(), T8nError> {
    write_json(
        &output_path(args, &args.output_alloc),
        &serde_json::to_value(post_alloc(pre_alloc, result))
            .map_err(|e| T8nError::Build(e.to_string()))?,
    )?;
    write_json(
        &output_path(args, &args.output_result),
        &result_json(result, rejections, block_exception),
    )?;
    write_body(&output_path(args, &args.output_body), result)
}

fn output_path(args: &T8nArgs, relative: &str) -> PathBuf {
    args.output_basedir.join(relative)
}

fn write_json(path: &Path, value: &Value) -> Result<(), T8nError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| T8nError::Io(path.display().to_string(), e))?;
    }
    let contents =
        serde_json::to_string_pretty(value).map_err(|e| T8nError::Build(e.to_string()))?;
    std::fs::write(path, contents).map_err(|e| T8nError::Io(path.display().to_string(), e))
}

fn write_body(path: &Path, result: &PayloadBuildResult) -> Result<(), T8nError> {
    let mut encoded = Vec::new();
    result.payload.body.transactions.encode(&mut encoded);
    std::fs::write(path, format!("0x{}", hex::encode(encoded)))
        .map_err(|e| T8nError::Io(path.display().to_string(), e))
}

/// The post-state alloc: the pre-state with the block's account updates
/// applied. The full pre-state was this run's input, so no trie iteration
/// is needed.
fn post_alloc(
    pre_alloc: &BTreeMap<Address, GenesisAccount>,
    result: &PayloadBuildResult,
) -> BTreeMap<Address, GenesisAccount> {
    let mut alloc = pre_alloc.clone();
    for update in &result.account_updates {
        if update.removed {
            alloc.remove(&update.address);
            if update.info.is_none() && update.code.is_none() && update.added_storage.is_empty() {
                continue;
            }
        }
        let entry = alloc
            .entry(update.address)
            .or_insert_with(|| GenesisAccount {
                code: Default::default(),
                storage: BTreeMap::new(),
                balance: U256::zero(),
                nonce: 0,
            });
        if update.removed_storage {
            entry.storage.clear();
        }
        if let Some(info) = &update.info {
            entry.balance = info.balance;
            entry.nonce = info.nonce;
        }
        if let Some(code) = &update.code {
            entry.code = code.code_bytes();
        }
        for (key, value) in &update.added_storage {
            let key = U256::from_big_endian(key.as_bytes());
            if value.is_zero() {
                entry.storage.remove(&key);
            } else {
                entry.storage.insert(key, *value);
            }
        }
    }
    alloc
}

fn hex_u64(value: u64) -> String {
    format!("{value:#x}")
}

fn hex_bytes(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn log_json(log: &Log) -> Value {
    json!({
        "address": log.address,
        "topics": log.topics,
        "data": hex_bytes(&log.data),
    })
}

fn receipt_json(
    index: usize,
    transaction: &Transaction,
    receipt: &Receipt,
    gas_used: u64,
) -> Value {
    json!({
        "type": hex_u64(receipt.tx_type as u64),
        "status": if receipt.succeeded { "0x1" } else { "0x0" },
        "cumulativeGasUsed": hex_u64(receipt.cumulative_gas_used),
        "logsBloom": bloom_from_logs(&receipt.logs, &NativeCrypto),
        "logs": receipt.logs.iter().map(log_json).collect::<Vec<_>>(),
        "transactionHash": transaction.hash(&NativeCrypto),
        "gasUsed": hex_u64(gas_used),
        "blockHash": H256::zero(),
        "transactionIndex": hex_u64(index as u64),
    })
}

fn result_json(
    result: &PayloadBuildResult,
    rejections: &[Rejection],
    block_exception: Option<String>,
) -> Value {
    let header = &result.payload.header;
    let transactions = &result.payload.body.transactions;

    let mut receipts = Vec::new();
    let mut logs: Vec<Log> = Vec::new();
    let mut previous_cumulative_gas = 0;
    for (index, (transaction, receipt)) in transactions.iter().zip(&result.receipts).enumerate() {
        let gas_used = receipt.cumulative_gas_used - previous_cumulative_gas;
        previous_cumulative_gas = receipt.cumulative_gas_used;
        receipts.push(receipt_json(index, transaction, receipt, gas_used));
        logs.extend(receipt.logs.iter().cloned());
    }
    let mut encoded_logs = Vec::new();
    logs.encode(&mut encoded_logs);
    let logs_hash = H256(NativeCrypto.keccak256(&encoded_logs));

    let mut output = json!({
        "stateRoot": header.state_root,
        "txRoot": header.transactions_root,
        "receiptsRoot": header.receipts_root,
        "logsHash": logs_hash,
        "logsBloom": bloom_from_logs(&logs, &NativeCrypto),
        "receipts": receipts,
        "rejected": rejections
            .iter()
            .map(|rejection| json!({"index": rejection.index, "error": rejection.error}))
            .collect::<Vec<_>>(),
        "gasUsed": hex_u64(header.gas_used),
    });
    let object = output
        .as_object_mut()
        .expect("result_json literal is an object");
    if let Some(base_fee) = header.base_fee_per_gas {
        object.insert("currentBaseFee".to_string(), json!(hex_u64(base_fee)));
    }
    if let Some(withdrawals_root) = header.withdrawals_root {
        object.insert("withdrawalsRoot".to_string(), json!(withdrawals_root));
    }
    if let Some(excess_blob_gas) = header.excess_blob_gas {
        object.insert(
            "currentExcessBlobGas".to_string(),
            json!(hex_u64(excess_blob_gas)),
        );
    }
    if let Some(blob_gas_used) = header.blob_gas_used {
        object.insert("blobGasUsed".to_string(), json!(hex_u64(blob_gas_used)));
    }
    if let Some(requests_hash) = header.requests_hash {
        object.insert("requestsHash".to_string(), json!(requests_hash));
        let requests: Vec<Value> = result
            .requests
            .iter()
            .filter(|request| request.0.len() > 1)
            .map(|request| json!(hex_bytes(&request.0)))
            .collect();
        object.insert("requests".to_string(), json!(requests));
    }
    if let Some(block_access_list) = &result.block_access_list {
        let mut encoded = Vec::new();
        block_access_list.encode(&mut encoded);
        object.insert("blockAccessList".to_string(), json!(hex_bytes(&encoded)));
    }
    if let Some(block_access_list_hash) = header.block_access_list_hash {
        object.insert(
            "blockAccessListHash".to_string(),
            json!(block_access_list_hash),
        );
    }
    if let Some(block_exception) = block_exception {
        object.insert("blockException".to_string(), json!(block_exception));
    }
    output
}
