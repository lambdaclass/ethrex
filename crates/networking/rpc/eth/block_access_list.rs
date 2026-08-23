use ethrex_common::types::block_access_list::{AccountChanges, BlockAccessList};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::encode::RLPEncode;
use serde_json::{Value, json};

use crate::{
    RpcApiContext, RpcErr, RpcHandler,
    types::block_identifier::{BlockIdentifier, BlockIdentifierOrHash, BlockTag},
};

pub struct BlockAccessListRequest {
    pub block: BlockIdentifierOrHash,
}

pub struct RawBlockAccessListRequest {
    pub block: BlockIdentifierOrHash,
}

/// Outcome of resolving a block identifier to its EIP-7928 block access list.
enum ResolvedBal {
    Found(BlockAccessList),
    /// The block is unknown, or the `pending` tag was requested.
    UnknownBlock,
}

fn parse_block_param(params: &Option<Vec<Value>>) -> Result<BlockIdentifierOrHash, RpcErr> {
    let params = params
        .as_ref()
        .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
    if params.is_empty() {
        return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
    }
    BlockIdentifierOrHash::parse(params[0].clone(), 0)
}

impl RpcHandler for BlockAccessListRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(BlockAccessListRequest {
            block: parse_block_param(params)?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        match resolve_bal(&self.block, &context).await? {
            ResolvedBal::Found(bal) => Ok(bal_to_json(&bal)),
            ResolvedBal::UnknownBlock => Ok(Value::Null),
        }
    }
}

impl RpcHandler for RawBlockAccessListRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(RawBlockAccessListRequest {
            block: parse_block_param(params)?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        // Unlike the JSON getter, the raw getter has no `null` result: an unknown
        // block is `-32001: Resource not found`.
        let bal = match resolve_bal(&self.block, &context).await? {
            ResolvedBal::Found(bal) => bal,
            ResolvedBal::UnknownBlock => {
                return Err(RpcErr::ResourceNotFound(format!(
                    "unknown block {}",
                    self.block
                )));
            }
        };
        Ok(Value::String(format!(
            "0x{}",
            hex::encode(bal.encode_to_vec())
        )))
    }
}

/// Resolves a block identifier to its block access list, per execution-apis
/// `eth_getBlockAccessList` / `debug_getRawBlockAccessList`: a block predating the
/// Amsterdam fork is `-32001: Resource not found`, and an Amsterdam+ block whose
/// access list can neither be served from the store nor regenerated is
/// `4444: Pruned history unavailable`.
async fn resolve_bal(
    block: &BlockIdentifierOrHash,
    context: &RpcApiContext,
) -> Result<ResolvedBal, RpcErr> {
    // A pending block has no access list to serve.
    if matches!(
        block,
        BlockIdentifierOrHash::Identifier(BlockIdentifier::Tag(BlockTag::Pending))
    ) {
        return Ok(ResolvedBal::UnknownBlock);
    }

    let header = match block {
        BlockIdentifierOrHash::Hash(hash) => context.storage.get_block_header_by_hash(*hash)?,
        BlockIdentifierOrHash::Identifier(id) => {
            match id.resolve_block_number(&context.storage).await? {
                Some(block_number) => context.storage.get_block_header(block_number)?,
                None => None,
            }
        }
    };
    let Some(header) = header else {
        return Ok(ResolvedBal::UnknownBlock);
    };

    if !context
        .storage
        .get_chain_config()
        .is_amsterdam_activated(header.timestamp)
    {
        return Err(RpcErr::ResourceNotFound(
            "block access lists start at the Amsterdam fork".to_owned(),
        ));
    }

    let block_hash = header.hash();
    let commitment = header.block_access_list_hash;

    // Fast path: serve from the BAL store populated at block import, but only
    // when it matches the header commitment (EIP-8159). A stale/empty stored
    // entry (e.g. from a prior regeneration against state later pruned) must
    // not be served; fall through to regeneration instead.
    if let Some(bal) = context.storage.get_block_access_list(block_hash)?
        && bal.matches_commitment(commitment, &NativeCrypto)
    {
        return Ok(ResolvedBal::Found(bal));
    }

    // Slow path: re-execute the block.
    let Some(full_block) = context.storage.get_block_by_hash(block_hash).await? else {
        return Err(RpcErr::PrunedHistoryUnavailable(format!(
            "block body for {block_hash:#x} is unavailable"
        )));
    };

    let bal = context
        .blockchain
        .generate_bal_for_block(&full_block)
        .map_err(|e| RpcErr::Internal(format!("Failed to generate BAL: {e}")))?;

    // Only serve a regenerated BAL that matches the header commitment; a
    // mismatch means it was re-executed against wrong/incomplete state.
    match bal.filter(|bal| bal.matches_commitment(commitment, &NativeCrypto)) {
        Some(bal) => Ok(ResolvedBal::Found(bal)),
        None => Err(RpcErr::PrunedHistoryUnavailable(format!(
            "block access list for {block_hash:#x} cannot be reconstructed"
        ))),
    }
}

/// Serializes a BlockAccessList into the JSON shape defined by execution-apis
/// `eth_getBlockAccessList` (EIP-7928): an array of AccountAccess objects with
/// camelCase fields and per-spec hex encodings (hash32 = full 32-byte hex,
/// quantities = no-leading-zero hex). Every entry carries all six fields, with
/// empty change lists encoded as empty arrays.
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
