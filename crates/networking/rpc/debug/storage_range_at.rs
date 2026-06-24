use std::collections::BTreeMap;

use ethrex_common::{Address, BigEndianHash, H256, utils::keccak};
use ethrex_storage::Store;
use ethrex_storage::error::StoreError;
use serde::Serialize;
use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler, types::block_identifier::BlockIdentifierOrHash};

/// `debug_storageRangeAt` — paginated iteration over an account's storage trie
/// at a block, matching the shape documented at
/// https://geth.ethereum.org/docs/interacting-with-geth/rpc/ns-debug#debug-storagerangeat.
///
/// Params: `[blockNrOrHash, txIndex, contractAddress, startKey, maxResult]`.
/// The first parameter accepts a block number, a tag (`latest` /
/// `earliest` / etc.), a 32-byte hash, or the EIP-1898 object form.
/// `startKey` is the hashed slot (`keccak256(slot_index)`), matching geth's
/// convention; the iterator yields slots in hashed-key order.
///
/// **Known divergences from geth**:
///
/// - `txIndex` is currently ignored — the response is always the state at the
///   *end* of the given block (equivalent to geth's `txIndex == -1`). Returning
///   the state immediately after the Nth transaction requires mid-block
///   trie-root reconstruction, which is a deferred feature.
/// - `StorageEntry.key` (the unhashed slot index) is always `null` because
///   ethrex has no preimage store.
/// - When the contract address has no account in the state trie at this block
///   (EOA, contract not yet deployed, or self-destructed), the response is
///   `{storage: {}, nextKey: null}` rather than an error — matches geth.
///
/// The trie traversal is wrapped in `tokio::task::spawn_blocking` because
/// `iter_storage_from` opens long-lived locked DB transactions and performs
/// synchronous trie I/O.
pub struct StorageRangeAtRequest {
    block: BlockIdentifierOrHash,
    /// Currently ignored — see type-level doc.
    tx_index: i64,
    address: Address,
    start_key: H256,
    max_result: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageRangeResult {
    storage: BTreeMap<H256, StorageEntry>,
    next_key: Option<H256>,
}

#[derive(Serialize)]
struct StorageEntry {
    /// Original (unhashed) slot index. Always `null` — no preimage store.
    key: Option<H256>,
    value: H256,
}

impl RpcHandler for StorageRangeAtRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 5 {
            return Err(RpcErr::BadParams(format!(
                "Expected 5 params, got {}",
                params.len()
            )));
        }
        let block = BlockIdentifierOrHash::parse(params[0].clone(), 0)?;
        let tx_index: i64 = serde_json::from_value(params[1].clone())
            .map_err(|e| RpcErr::BadParams(format!("invalid txIndex: {e}")))?;
        let address: Address = serde_json::from_value(params[2].clone())
            .map_err(|e| RpcErr::BadParams(format!("invalid contractAddress: {e}")))?;
        let start_key: H256 = serde_json::from_value(params[3].clone())
            .map_err(|e| RpcErr::BadParams(format!("invalid startKey: {e}")))?;
        let max_result: usize = serde_json::from_value(params[4].clone())
            .map_err(|e| RpcErr::BadParams(format!("invalid maxResult: {e}")))?;
        if max_result == 0 {
            return Err(RpcErr::BadParams(
                "maxResult must be greater than 0".to_owned(),
            ));
        }
        Ok(StorageRangeAtRequest {
            block,
            tx_index,
            address,
            start_key,
            max_result,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let header = self
            .block
            .resolve_block_header(&context.storage)
            .await?
            .ok_or_else(|| RpcErr::WrongParam("Block not found".to_string()))?;

        // `tx_index` is accepted (geth's `-1` sentinel works since it's `i64`)
        // but not honoured — see type-level doc.
        let _ = self.tx_index;

        let state_root = header.state_root;
        let hashed_address = keccak(self.address);
        let start_key = self.start_key;
        let max_result = self.max_result;
        let storage = context.storage.clone();

        let (entries, next_key) = tokio::task::spawn_blocking(move || {
            collect_storage_range(&storage, state_root, hashed_address, start_key, max_result)
        })
        .await
        .map_err(|e| RpcErr::Internal(format!("storageRangeAt task failed: {e}")))??;

        Ok(serde_json::to_value(StorageRangeResult {
            storage: entries,
            next_key,
        })?)
    }
}

fn collect_storage_range(
    storage: &Store,
    state_root: H256,
    hashed_address: H256,
    start_key: H256,
    max_result: usize,
) -> Result<(BTreeMap<H256, StorageEntry>, Option<H256>), StoreError> {
    let Some(iter) = storage.iter_storage_from(state_root, hashed_address, start_key)? else {
        // Account not present in state (EOA, undeployed, or destructed).
        return Ok((BTreeMap::new(), None));
    };
    let mut entries = BTreeMap::new();
    let mut next_key = None;
    for (hashed_slot, value) in iter {
        if entries.len() >= max_result {
            next_key = Some(hashed_slot);
            break;
        }
        entries.insert(
            hashed_slot,
            StorageEntry {
                key: None,
                value: H256::from_uint(&value),
            },
        );
    }
    Ok((entries, next_key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RpcHandler;
    use crate::types::block_identifier::BlockIdentifier;
    use serde_json::json;

    #[test]
    fn parse_valid_params_by_hash() {
        let params = Some(vec![
            json!("0x0000000000000000000000000000000000000000000000000000000000000001"),
            json!(0),
            json!("0x0000000000000000000000000000000000000001"),
            json!("0x0000000000000000000000000000000000000000000000000000000000000000"),
            json!(128),
        ]);
        let req = StorageRangeAtRequest::parse(&params).unwrap();
        assert!(matches!(req.block, BlockIdentifierOrHash::Hash(_)));
        assert_eq!(req.tx_index, 0);
        assert_eq!(req.address, Address::from_low_u64_be(1));
        assert_eq!(req.start_key, H256::zero());
        assert_eq!(req.max_result, 128);
    }

    #[test]
    fn parse_valid_params_by_number_and_neg_tx_index() {
        let params = Some(vec![
            json!("0xa"),
            json!(-1),
            json!("0x0000000000000000000000000000000000000001"),
            json!("0x0000000000000000000000000000000000000000000000000000000000000000"),
            json!(128),
        ]);
        let req = StorageRangeAtRequest::parse(&params).unwrap();
        assert!(matches!(
            req.block,
            BlockIdentifierOrHash::Identifier(BlockIdentifier::Number(10))
        ));
        assert_eq!(req.tx_index, -1);
    }

    #[test]
    fn parse_valid_params_by_tag() {
        let params = Some(vec![
            json!("latest"),
            json!(0),
            json!("0x0000000000000000000000000000000000000001"),
            json!("0x0000000000000000000000000000000000000000000000000000000000000000"),
            json!(128),
        ]);
        let req = StorageRangeAtRequest::parse(&params).unwrap();
        assert!(matches!(req.block, BlockIdentifierOrHash::Identifier(_)));
    }

    #[test]
    fn parse_no_params() {
        assert!(StorageRangeAtRequest::parse(&None).is_err());
    }

    #[test]
    fn parse_wrong_param_count() {
        let params = Some(vec![json!("0x01"), json!(0)]);
        assert!(StorageRangeAtRequest::parse(&params).is_err());
    }

    #[test]
    fn parse_too_many_params() {
        let params = Some(vec![
            json!("0x0000000000000000000000000000000000000000000000000000000000000001"),
            json!(0),
            json!("0x0000000000000000000000000000000000000001"),
            json!("0x0000000000000000000000000000000000000000000000000000000000000000"),
            json!(128),
            json!("extra"),
        ]);
        assert!(StorageRangeAtRequest::parse(&params).is_err());
    }
}
