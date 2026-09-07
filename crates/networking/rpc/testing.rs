use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::payload::{BuildPayloadArgs, create_payload};
use ethrex_common::types::{ELASTICITY_MULTIPLIER, Transaction};
use ethrex_common::{Bytes, H256};
use ethrex_rlp::error::RLPDecodeError;
use serde_json::Value;

use crate::types::fork_choice::PayloadAttributesV3;
use crate::types::payload::{ExecutionPayload, ExecutionPayloadResponse};
use crate::{RpcApiContext, RpcErr, RpcHandler};

/// `testing_buildBlockV1`: builds a block on top of `parent_block_hash` using the
/// provided payload attributes and (optionally) an explicit transaction list.
///
/// This is a testing-only method for generating fixtures. It does NOT modify the
/// canonical chain or the head block; it only builds and returns a payload.
pub struct BuildBlockV1Request {
    pub parent_block_hash: H256,
    pub attributes: PayloadAttributesV3,
    /// `None` (JSON `null`) -> build from the mempool.
    /// `Some([])` -> build an empty block.
    /// `Some([..])` -> include exactly these transactions, in order, no mempool txs.
    pub transactions: Option<Vec<Transaction>>,
    /// Override for the block's `extraData`. Defaults to empty when omitted.
    pub extra_data: Bytes,
    /// `slotNumber` from the payload attributes (EIP-7843). `None` when omitted;
    /// only applied to the built header when Amsterdam is active.
    pub slot_number: Option<u64>,
}

/// Decodes a `0x`-prefixed hex string JSON value into raw bytes.
fn decode_hex_value(value: &Value) -> Result<Bytes, RpcErr> {
    let str_data = serde_json::from_value::<String>(value.clone())?;
    let str_data = str_data
        .strip_prefix("0x")
        .ok_or_else(|| RpcErr::BadParams("hex value is not 0x-prefixed".to_owned()))?;
    hex::decode(str_data)
        .map(Bytes::from)
        .map_err(|err| RpcErr::BadParams(err.to_string()))
}

/// Decodes a `0x`-prefixed hex string JSON value into a `u64`.
fn decode_hex_u64(value: &Value) -> Result<u64, RpcErr> {
    let str_data = serde_json::from_value::<String>(value.clone())?;
    let str_data = str_data.strip_prefix("0x").unwrap_or(&str_data);
    u64::from_str_radix(str_data, 16)
        .map_err(|err| RpcErr::BadParams(format!("invalid slotNumber: {err}")))
}

fn decode_transactions(value: &Value) -> Result<Option<Vec<Transaction>>, RpcErr> {
    if value.is_null() {
        return Ok(None);
    }
    let raw_txs = value
        .as_array()
        .ok_or_else(|| RpcErr::BadParams("transactions must be an array or null".to_owned()))?;
    let txs = raw_txs
        .iter()
        .map(|raw| {
            let bytes = decode_hex_value(raw)?;
            Transaction::decode_canonical(&bytes).map_err(|err: RLPDecodeError| {
                RpcErr::BadParams(format!("invalid transaction: {err}"))
            })
        })
        .collect::<Result<Vec<_>, RpcErr>>()?;
    Ok(Some(txs))
}

impl RpcHandler for BuildBlockV1Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 3 && params.len() != 4 {
            return Err(RpcErr::BadParams(format!(
                "Expected 3 or 4 params and {} were provided",
                params.len()
            )));
        }

        let parent_block_hash: H256 = serde_json::from_value(params[0].clone())?;
        let attributes: PayloadAttributesV3 = serde_json::from_value(params[1].clone())?;
        let transactions = decode_transactions(&params[2])?;
        let extra_data = match params.get(3) {
            Some(value) if !value.is_null() => decode_hex_value(value)?,
            _ => Bytes::new(),
        };
        // Block validation rejects headers with `extra_data` longer than 32 bytes
        // (crates/common/types/block.rs); reject it here so the method never
        // returns a payload that is invalid by consensus rules.
        if extra_data.len() > 32 {
            return Err(RpcErr::BadParams(format!(
                "extraData exceeds 32 bytes (got {})",
                extra_data.len()
            )));
        }
        // `PayloadAttributesV3` has no `slotNumber`, so read it from the raw
        // attributes object (EIP-7843, present from Amsterdam onwards).
        let slot_number = match params[1].get("slotNumber") {
            Some(value) if !value.is_null() => Some(decode_hex_u64(value)?),
            _ => None,
        };

        Ok(Self {
            parent_block_hash,
            attributes,
            transactions,
            extra_data,
            slot_number,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        // The block must be built on top of the current canonical head; this both
        // matches geth's behaviour and guarantees the parent state is available.
        let head = context
            .storage
            .get_latest_canonical_block_hash()
            .await?
            .ok_or_else(|| RpcErr::Internal("no canonical head block".to_string()))?;
        if head != self.parent_block_hash {
            return Err(RpcErr::BadParams(
                "parentBlockHash is not the current head".to_string(),
            ));
        }

        let parent_header = context
            .storage
            .get_block_header_by_hash(self.parent_block_hash)
            .map_err(|err| RpcErr::Internal(err.to_string()))?
            .ok_or_else(|| RpcErr::BadParams("parent block not found".to_string()))?;
        if self.attributes.timestamp <= parent_header.timestamp {
            return Err(RpcErr::InvalidPayloadAttributes(
                "invalid timestamp".to_string(),
            ));
        }

        let chain_config = context.storage.get_chain_config();
        // EIP-7843 requires the block header to carry a `slot_number` from
        // Amsterdam onwards; mirror genesis behaviour by defaulting to 0 when
        // the attribute is absent. Pre-Amsterdam blocks leave it unset.
        let slot_number = chain_config
            .is_amsterdam_activated(self.attributes.timestamp)
            .then(|| self.slot_number.unwrap_or(0));

        let args = BuildPayloadArgs {
            parent: self.parent_block_hash,
            timestamp: self.attributes.timestamp,
            fee_recipient: self.attributes.suggested_fee_recipient,
            random: self.attributes.prev_randao,
            withdrawals: self.attributes.withdrawals.clone(),
            beacon_root: self.attributes.parent_beacon_block_root,
            slot_number,
            version: 3,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            gas_ceil: context.gas_ceil,
        };

        let payload = match create_payload(&args, &context.storage, self.extra_data.clone()) {
            Ok(payload) => payload,
            Err(ethrex_blockchain::error::ChainError::EvmError(error)) => return Err(error.into()),
            Err(error) => return Err(RpcErr::Internal(error.to_string())),
        };

        let result = match self.transactions.clone() {
            None => context.blockchain.build_payload(payload),
            Some(transactions) => context
                .blockchain
                .build_payload_with_transactions(payload, transactions),
        }
        // Failures from a caller-supplied transaction list are user input, not
        // server bugs: map them to BadParams (-32000) so a JSON-RPC client can
        // distinguish a bad transaction from an internal failure (-32603).
        .map_err(|err| match err {
            ChainError::EvmError(error) => error.into(),
            ChainError::InvalidTransaction(msg) => {
                RpcErr::BadParams(format!("invalid transaction: {msg}"))
            }
            ChainError::Custom(msg) => RpcErr::BadParams(msg),
            other => RpcErr::Internal(other.to_string()),
        })?;

        let timestamp = result.payload.header.timestamp;

        let block_access_list = chain_config
            .is_amsterdam_activated(timestamp)
            .then(|| result.block_access_list.clone())
            .flatten();
        let execution_requests = chain_config.is_prague_activated(timestamp).then(|| {
            result
                .requests
                .iter()
                .filter(|r| !r.is_empty())
                .cloned()
                .collect()
        });

        let response = ExecutionPayloadResponse {
            execution_payload: ExecutionPayload::from_block(result.payload, block_access_list),
            block_value: result.block_value,
            blobs_bundle: Some(result.blobs_bundle),
            should_override_builder: Some(false),
            execution_requests,
        };

        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{default_context_with_storage, setup_store};
    use serde_json::json;

    async fn head_hash_and_timestamp(context: &RpcApiContext) -> (H256, u64) {
        let hash = context
            .storage
            .get_latest_canonical_block_hash()
            .await
            .unwrap()
            .unwrap();
        let header = context
            .storage
            .get_block_header_by_hash(hash)
            .unwrap()
            .unwrap();
        (hash, header.timestamp)
    }

    fn build_params(parent: H256, timestamp: u64, transactions: Value) -> Option<Vec<Value>> {
        Some(vec![
            json!(format!("{parent:#x}")),
            json!({
                "timestamp": format!("{timestamp:#x}"),
                "prevRandao": format!("{:#x}", H256::zero()),
                "suggestedFeeRecipient": "0x0000000000000000000000000000000000000000",
                "withdrawals": [],
                "parentBeaconBlockRoot": format!("{:#x}", H256::zero()),
            }),
            transactions,
        ])
    }

    #[test]
    fn parse_accepts_null_and_array_transactions() {
        let parent = H256::zero();
        let null_req =
            BuildBlockV1Request::parse(&build_params(parent, 0x10, Value::Null)).unwrap();
        assert!(null_req.transactions.is_none());

        let empty_req = BuildBlockV1Request::parse(&build_params(parent, 0x10, json!([]))).unwrap();
        assert_eq!(empty_req.transactions.as_ref().unwrap().len(), 0);
        assert!(empty_req.extra_data.is_empty());
        assert!(empty_req.slot_number.is_none());
    }

    #[test]
    fn parse_reads_slot_number_from_attributes() {
        let mut params = build_params(H256::zero(), 0x10, json!([])).unwrap();
        params[1]["slotNumber"] = json!("0x2a");
        let req = BuildBlockV1Request::parse(&Some(params)).unwrap();
        assert_eq!(req.slot_number, Some(0x2a));
    }

    #[tokio::test]
    async fn rejects_parent_that_is_not_head() {
        let store = setup_store().await;
        let context = default_context_with_storage(store).await;
        let (_, timestamp) = head_hash_and_timestamp(&context).await;

        let params = build_params(H256::from_low_u64_be(1234), timestamp + 12, json!([]));
        let err = BuildBlockV1Request::parse(&params)
            .unwrap()
            .handle(context)
            .await
            .unwrap_err();
        assert!(matches!(err, RpcErr::BadParams(_)));
    }

    #[tokio::test]
    async fn builds_empty_block_on_head() {
        let store = setup_store().await;
        let context = default_context_with_storage(store).await;
        let (head, timestamp) = head_hash_and_timestamp(&context).await;

        let params = build_params(head, timestamp + 12, json!([]));
        let response = BuildBlockV1Request::parse(&params)
            .unwrap()
            .handle(context)
            .await
            .unwrap();

        let payload = &response["executionPayload"];
        assert_eq!(payload["parentHash"], json!(format!("{head:#x}")));
        assert_eq!(payload["blockNumber"], json!("0x1"));
        assert_eq!(payload["transactions"], json!([]));
        assert_eq!(response["shouldOverrideBuilder"], json!(false));
    }
}
