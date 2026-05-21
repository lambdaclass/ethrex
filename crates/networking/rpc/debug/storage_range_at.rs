use std::collections::BTreeMap;

use ethrex_common::{Address, BigEndianHash, H256};
use serde::Serialize;
use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler};

pub struct StorageRangeAtRequest {
    block_hash: H256,
    tx_index: usize,
    address: Address,
    start_key: H256,
    max_result: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageRangeResult {
    storage: BTreeMap<H256, StorageEntry>,
    #[serde(rename = "nextKey")]
    next_key: Option<H256>,
}

#[derive(Serialize)]
struct StorageEntry {
    /// The original (unhashed) key. Null when the preimage is not available.
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
        let block_hash: H256 = serde_json::from_value(params[0].clone())?;
        let tx_index: usize = serde_json::from_value(params[1].clone())?;
        let address: Address = serde_json::from_value(params[2].clone())?;
        let start_key: H256 = serde_json::from_value(params[3].clone())?;
        let max_result: usize = serde_json::from_value(params[4].clone())?;
        Ok(StorageRangeAtRequest {
            block_hash,
            tx_index,
            address,
            start_key,
            max_result,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        // Get the block header to obtain the state root.
        // Note: txIndex-based state reconstruction (re-executing up to txIndex)
        // is not yet supported; we use the block's final state root.
        let header = context
            .storage
            .get_block_header_by_hash(self.block_hash)?
            .ok_or(RpcErr::Internal("Block not found".to_string()))?;

        let _ = self.tx_index; // TODO: re-execute up to tx_index for precise state

        let hashed_address = ethrex_common::utils::keccak(self.address);

        let iter = context
            .storage
            .iter_storage_from(header.state_root, hashed_address, self.start_key)
            .map_err(|e| RpcErr::Internal(e.to_string()))?;

        let Some(iter) = iter else {
            // Account not found in state
            return Ok(serde_json::to_value(StorageRangeResult {
                storage: BTreeMap::new(),
                next_key: None,
            })?);
        };

        let mut storage = BTreeMap::new();
        let mut next_key = None;

        for (hashed_slot, value) in iter {
            if storage.len() >= self.max_result {
                next_key = Some(hashed_slot);
                break;
            }
            storage.insert(
                hashed_slot,
                StorageEntry {
                    // ethrex does not store keccak preimages, so key is unavailable
                    key: None,
                    value: H256::from_uint(&value),
                },
            );
        }

        Ok(serde_json::to_value(StorageRangeResult {
            storage,
            next_key,
        })?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RpcHandler;
    use serde_json::json;

    #[test]
    fn parse_valid_params() {
        let params = Some(vec![
            json!("0x0000000000000000000000000000000000000000000000000000000000000001"),
            json!(0),
            json!("0x0000000000000000000000000000000000000001"),
            json!("0x0000000000000000000000000000000000000000000000000000000000000000"),
            json!(128),
        ]);
        let req = StorageRangeAtRequest::parse(&params).unwrap();
        assert_eq!(req.block_hash, H256::from_low_u64_be(1));
        assert_eq!(req.tx_index, 0);
        assert_eq!(req.max_result, 128);
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
