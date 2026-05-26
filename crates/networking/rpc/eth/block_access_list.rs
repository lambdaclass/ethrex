use ethrex_common::types::block_access_list::{AccountChanges, BlockAccessList};
use serde_json::{Value, json};

use crate::{RpcApiContext, RpcErr, RpcHandler, types::block_identifier::BlockIdentifierOrHash};

pub struct BlockAccessListRequest {
    pub block: BlockIdentifierOrHash,
}

impl RpcHandler for BlockAccessListRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.is_empty() {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        }
        let block = BlockIdentifierOrHash::parse(params[0].clone(), 0)?;
        Ok(BlockAccessListRequest { block })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        // Per execution-apis, unknown blocks map to the `notFound` schema (null).
        let block_hash = match &self.block {
            BlockIdentifierOrHash::Hash(h) => *h,
            BlockIdentifierOrHash::Identifier(id) => {
                let Some(block_number) = id.resolve_block_number(&context.storage).await? else {
                    return Ok(Value::Null);
                };
                let Some(header) = context.storage.get_block_header(block_number)? else {
                    return Ok(Value::Null);
                };
                header.hash()
            }
        };

        // Fast path: serve from the BAL store populated at block import.
        // Avoids re-executing the block when it's already known.
        if let Some(bal) = context.storage.get_block_access_list(block_hash)? {
            return Ok(bal_to_json(&bal));
        }

        // Slow path: re-execute the block. Returns None for pre-Amsterdam blocks.
        let Some(block) = context.storage.get_block_by_hash(block_hash).await? else {
            return Ok(Value::Null);
        };

        let bal = context
            .blockchain
            .generate_bal_for_block(&block)
            .map_err(|e| RpcErr::Internal(format!("Failed to generate BAL: {e}")))?;

        match bal {
            Some(bal) => Ok(bal_to_json(&bal)),
            None => Ok(Value::Null),
        }
    }
}

/// Serializes a BlockAccessList into the JSON shape defined by execution-apis
/// `eth_getBlockAccessList` (EIP-7928): an array of AccountAccess objects with
/// camelCase fields and per-spec hex encodings (hash32 = full 32-byte hex,
/// quantities = no-leading-zero hex).
fn bal_to_json(bal: &BlockAccessList) -> Value {
    Value::Array(bal.accounts().iter().map(account_to_json).collect())
}

fn account_to_json(acc: &AccountChanges) -> Value {
    let storage_changes: Vec<Value> = acc
        .storage_changes
        .iter()
        .map(|sc| {
            let changes: Vec<Value> = sc
                .slot_changes
                .iter()
                .map(|c| {
                    json!({
                        "index": format!("{:#x}", c.block_access_index),
                        "value": format!("0x{:064x}", c.post_value),
                    })
                })
                .collect();
            json!({
                "key": format!("0x{:064x}", sc.slot),
                "changes": changes,
            })
        })
        .collect();

    let storage_reads: Vec<Value> = acc
        .storage_reads
        .iter()
        .map(|slot| Value::String(format!("0x{:064x}", slot)))
        .collect();

    let balance_changes: Vec<Value> = acc
        .balance_changes
        .iter()
        .map(|bc| {
            json!({
                "index": format!("{:#x}", bc.block_access_index),
                "value": format!("{:#x}", bc.post_balance),
            })
        })
        .collect();

    let nonce_changes: Vec<Value> = acc
        .nonce_changes
        .iter()
        .map(|nc| {
            json!({
                "index": format!("{:#x}", nc.block_access_index),
                "value": format!("{:#x}", nc.post_nonce),
            })
        })
        .collect();

    let code_changes: Vec<Value> = acc
        .code_changes
        .iter()
        .map(|cc| {
            json!({
                "index": format!("{:#x}", cc.block_access_index),
                "code": format!("0x{}", hex::encode(&cc.new_code)),
            })
        })
        .collect();

    json!({
        "address": format!("{:#x}", acc.address),
        "storageChanges": storage_changes,
        "storageReads": storage_reads,
        "balanceChanges": balance_changes,
        "nonceChanges": nonce_changes,
        "codeChanges": code_changes,
    })
}
