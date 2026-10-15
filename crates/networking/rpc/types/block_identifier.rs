use std::{fmt::Display, str::FromStr};

use ethrex_common::types::{BlockHash, BlockHeader, BlockNumber};
use ethrex_storage::{Store, error::StoreError};
use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};

use crate::utils::RpcErr;

/// Longest raw JSON accepted for a block identifier param: a tag, a hex number, or an
/// EIP-1898 object with a hash and `requireCanonical`, with ample room for whitespace.
/// Anything longer is rejected before being turned into a `Value`.
const MAX_BLOCK_IDENTIFIER_PARAM_LEN: usize = 1024;

fn block_identifier_param_value(param: &RawValue, arg_index: u64) -> Result<Value, RpcErr> {
    if param.get().len() > MAX_BLOCK_IDENTIFIER_PARAM_LEN {
        return Err(RpcErr::BadParams(format!(
            "block identifier param {arg_index} is too long"
        )));
    }
    Ok(serde_json::from_str(param.get())?)
}

#[derive(Clone, Debug)]
pub enum BlockIdentifier {
    Number(BlockNumber),
    Tag(BlockTag),
}

#[derive(Clone, Debug)]
pub enum BlockIdentifierOrHash {
    Hash(BlockHash),
    Identifier(BlockIdentifier),
}

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum BlockTag {
    Earliest,
    Finalized,
    Safe,
    #[default]
    Latest,
    Pending,
}

impl BlockIdentifier {
    pub async fn resolve_block_number(
        &self,
        storage: &Store,
    ) -> Result<Option<BlockNumber>, StoreError> {
        match self {
            BlockIdentifier::Number(num) => Ok(Some(*num)),
            BlockIdentifier::Tag(tag) => match tag {
                BlockTag::Earliest => Ok(Some(storage.get_earliest_block_number().await?)),
                BlockTag::Finalized => storage.get_finalized_block_number().await,
                BlockTag::Safe => storage.get_safe_block_number().await,
                BlockTag::Latest => Ok(Some(storage.get_latest_block_number()?)),
                BlockTag::Pending => {
                    // TODO(#1112): We need to check individual intrincacies of the pending tag for
                    // each RPC method that uses it.
                    if let Some(pending_block_number) = storage.get_pending_block_number().await? {
                        Ok(Some(pending_block_number))
                    } else {
                        // If there are no pending blocks, we return the latest block number
                        Ok(Some(storage.get_latest_block_number()?))
                    }
                }
            },
        }
    }

    /// Parses the raw JSON-RPC param at `arg_index`.
    pub fn parse_param(param: &RawValue, arg_index: u64) -> Result<Self, RpcErr> {
        Self::parse(block_identifier_param_value(param, arg_index)?, arg_index)
    }

    pub fn parse(serde_value: Value, arg_index: u64) -> Result<Self, RpcErr> {
        // Check if it is a BlockTag
        if let Ok(tag) = serde_json::from_value::<BlockTag>(serde_value.clone()) {
            return Ok(BlockIdentifier::Tag(tag));
        };
        // Parse BlockNumber
        let hex_str = match serde_json::from_value::<String>(serde_value) {
            Ok(hex_str) => hex_str,
            Err(error) => return Err(RpcErr::BadParams(error.to_string())),
        };
        // Check that the BlockNumber is 0x prefixed
        let Some(hex_str) = hex_str.strip_prefix("0x") else {
            return Err(RpcErr::BadHexFormat(arg_index));
        };

        // Parse hex string
        let Ok(block_number) = u64::from_str_radix(hex_str, 16) else {
            return Err(RpcErr::BadHexFormat(arg_index));
        };
        Ok(BlockIdentifier::Number(block_number))
    }

    pub async fn resolve_block_header(
        &self,
        storage: &Store,
    ) -> Result<Option<BlockHeader>, StoreError> {
        match self.resolve_block_number(storage).await? {
            Some(block_number) => storage.get_block_header(block_number),
            _ => Ok(None),
        }
    }
}

impl BlockIdentifierOrHash {
    /// Header the call should execute against.
    ///
    /// A hash names that block, including one that is no longer canonical.
    /// Resolving it to a height and then calling `get_block_header` would load
    /// the canonical sibling: `get_block_header` reads `CANONICAL_BLOCK_HASHES`.
    /// `eth_call` and `eth_estimateGas` copy `slot_number` from this header, so
    /// that substitution makes `SLOTNUM` return the canonical slot.
    pub async fn resolve_block_header(
        &self,
        storage: &Store,
    ) -> Result<Option<BlockHeader>, StoreError> {
        match self {
            BlockIdentifierOrHash::Hash(block_hash) => {
                storage.get_block_header_by_hash(*block_hash)
            }
            BlockIdentifierOrHash::Identifier(id) => id.resolve_block_header(storage).await,
        }
    }

    pub async fn resolve_block_number(
        &self,
        storage: &Store,
    ) -> Result<Option<BlockNumber>, StoreError> {
        match self {
            BlockIdentifierOrHash::Identifier(id) => id.resolve_block_number(storage).await,
            BlockIdentifierOrHash::Hash(block_hash) => storage.get_block_number(*block_hash).await,
        }
    }

    /// Parses the raw JSON-RPC param at `arg_index`.
    pub fn parse_param(param: &RawValue, arg_index: u64) -> Result<Self, RpcErr> {
        Self::parse(block_identifier_param_value(param, arg_index)?, arg_index)
    }

    pub fn parse(serde_value: Value, arg_index: u64) -> Result<BlockIdentifierOrHash, RpcErr> {
        // EIP-1898 object form: {"blockHash": "0x.."} or {"blockNumber": "0x.." | tag}.
        // `requireCanonical` is accepted but not enforced (matches geth's permissive behavior).
        if let Value::Object(map) = &serde_value {
            if map.contains_key("blockHash") && map.contains_key("blockNumber") {
                return Err(RpcErr::BadParams(
                    "EIP-1898 block identifier cannot specify both `blockHash` and `blockNumber`"
                        .to_string(),
                ));
            }
            if let Some(hash_value) = map.get("blockHash") {
                let hex_str = serde_json::from_value::<String>(hash_value.clone())
                    .map_err(|e| RpcErr::BadParams(e.to_string()))?;
                let block_hash =
                    BlockHash::from_str(&hex_str).map_err(|_| RpcErr::BadHexFormat(arg_index))?;
                return Ok(BlockIdentifierOrHash::Hash(block_hash));
            }
            if let Some(number_value) = map.get("blockNumber") {
                return BlockIdentifier::parse(number_value.clone(), arg_index)
                    .map(BlockIdentifierOrHash::Identifier);
            }
            return Err(RpcErr::BadParams(
                "EIP-1898 block identifier requires `blockHash` or `blockNumber`".to_string(),
            ));
        }

        // Parse as BlockHash
        if let Some(block_hash) = serde_json::from_value::<String>(serde_value.clone())
            .ok()
            .and_then(|hex_str| BlockHash::from_str(&hex_str).ok())
        {
            Ok(BlockIdentifierOrHash::Hash(block_hash))
        } else {
            // Parse as BlockIdentifier
            BlockIdentifier::parse(serde_value, arg_index).map(BlockIdentifierOrHash::Identifier)
        }
    }

    #[allow(unused)]
    pub async fn is_latest(&self, storage: &Store) -> Result<bool, StoreError> {
        if self == &BlockTag::Latest {
            return Ok(true);
        }

        let result = self.resolve_block_number(storage).await?;
        let latest = storage.get_latest_block_number()?;

        Ok(result.is_some_and(|res| res == latest))
    }
}

impl From<BlockIdentifier> for Value {
    fn from(value: BlockIdentifier) -> Self {
        match value {
            BlockIdentifier::Number(n) => json!(format!("{n:#x}")),
            BlockIdentifier::Tag(tag) => match tag {
                BlockTag::Earliest => json!("earliest"),
                BlockTag::Finalized => json!("finalized"),
                BlockTag::Safe => json!("safe"),
                BlockTag::Latest => json!("latest"),
                BlockTag::Pending => json!("pending"),
            },
        }
    }
}

impl Display for BlockIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockIdentifier::Number(num) => num.fmt(f),
            BlockIdentifier::Tag(tag) => match tag {
                BlockTag::Earliest => "earliest".fmt(f),
                BlockTag::Finalized => "finalized".fmt(f),
                BlockTag::Safe => "safe".fmt(f),
                BlockTag::Latest => "latest".fmt(f),
                BlockTag::Pending => "pending".fmt(f),
            },
        }
    }
}

impl Default for BlockIdentifier {
    fn default() -> BlockIdentifier {
        BlockIdentifier::Tag(BlockTag::default())
    }
}

impl Display for BlockIdentifierOrHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockIdentifierOrHash::Identifier(id) => id.fmt(f),
            BlockIdentifierOrHash::Hash(hash) => hash.fmt(f),
        }
    }
}

impl PartialEq<BlockTag> for BlockIdentifierOrHash {
    fn eq(&self, other: &BlockTag) -> bool {
        match self {
            BlockIdentifierOrHash::Identifier(BlockIdentifier::Tag(tag)) => tag == other,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_eip1898_block_hash_object() {
        let hash = "0x32a2b8016bfefb8a25030cbc6636a833584bd6ae0db2e2db7176f27a431c5563";
        let parsed = BlockIdentifierOrHash::parse(json!({"blockHash": hash}), 0).unwrap();
        match parsed {
            BlockIdentifierOrHash::Hash(h) => assert_eq!(format!("{h:#x}"), hash),
            other => panic!("expected Hash, got {other:?}"),
        }
    }

    #[test]
    fn parse_eip1898_block_hash_object_with_require_canonical() {
        let hash = "0x32a2b8016bfefb8a25030cbc6636a833584bd6ae0db2e2db7176f27a431c5563";
        let parsed =
            BlockIdentifierOrHash::parse(json!({"blockHash": hash, "requireCanonical": true}), 0)
                .unwrap();
        assert!(matches!(parsed, BlockIdentifierOrHash::Hash(_)));
    }

    #[test]
    fn parse_eip1898_block_number_object_hex() {
        let parsed = BlockIdentifierOrHash::parse(json!({"blockNumber": "0x10"}), 0).unwrap();
        match parsed {
            BlockIdentifierOrHash::Identifier(BlockIdentifier::Number(n)) => assert_eq!(n, 16),
            other => panic!("expected Number(16), got {other:?}"),
        }
    }

    #[test]
    fn parse_eip1898_block_number_object_tag() {
        let parsed = BlockIdentifierOrHash::parse(json!({"blockNumber": "latest"}), 0).unwrap();
        assert_eq!(parsed, BlockTag::Latest);
    }

    #[test]
    fn parse_eip1898_object_missing_keys_fails() {
        let err = BlockIdentifierOrHash::parse(json!({}), 0).unwrap_err();
        assert!(matches!(err, RpcErr::BadParams(_)));
    }

    #[test]
    fn parse_eip1898_object_both_keys_fails() {
        let value = json!({
            "blockHash": "0x32a2b8016bfefb8a25030cbc6636a833584bd6ae0db2e2db7176f27a431c5563",
            "blockNumber": "0x10",
        });
        let err = BlockIdentifierOrHash::parse(value, 0).unwrap_err();
        assert!(matches!(err, RpcErr::BadParams(_)));
    }

    #[test]
    fn parse_string_forms_still_work() {
        let hash = "0x32a2b8016bfefb8a25030cbc6636a833584bd6ae0db2e2db7176f27a431c5563";
        assert!(matches!(
            BlockIdentifierOrHash::parse(json!(hash), 0).unwrap(),
            BlockIdentifierOrHash::Hash(_)
        ));
        assert!(matches!(
            BlockIdentifierOrHash::parse(json!("0x10"), 0).unwrap(),
            BlockIdentifierOrHash::Identifier(BlockIdentifier::Number(16))
        ));
        assert_eq!(
            BlockIdentifierOrHash::parse(json!("latest"), 0).unwrap(),
            BlockTag::Latest
        );
    }

    fn header_at(number: u64, slot: u64) -> BlockHeader {
        // Earlier optional header fields have to be present. RLP decodes trailing
        // optionals greedily, so a bare `slot_number` is read as `base_fee_per_gas`.
        BlockHeader {
            number,
            base_fee_per_gas: Some(1),
            withdrawals_root: Some(ethrex_common::H256::zero()),
            blob_gas_used: Some(0),
            excess_blob_gas: Some(0),
            parent_beacon_block_root: Some(ethrex_common::H256::zero()),
            requests_hash: Some(ethrex_common::H256::zero()),
            block_access_list_hash: Some(ethrex_common::H256::zero()),
            slot_number: Some(slot),
            ..Default::default()
        }
    }

    /// Two headers share a height. The canonical one must not answer a query for the other hash.
    #[tokio::test]
    async fn hash_resolves_to_that_header_not_the_canonical_sibling() {
        use ethrex_storage::EngineType;

        let storage = Store::new("slotnum-hash", EngineType::InMemory).expect("store");
        let canonical = header_at(7, 4);
        let side = header_at(7, 9);
        storage
            .add_block_headers(vec![canonical.clone(), side.clone()])
            .await
            .expect("headers");
        storage
            .forkchoice_update(vec![(7, canonical.hash())], 7, canonical.hash(), None, None)
            .await
            .expect("forkchoice");

        let by_side = BlockIdentifierOrHash::Hash(side.hash())
            .resolve_block_header(&storage)
            .await
            .expect("resolve side")
            .expect("side header");
        assert_eq!(by_side.hash(), side.hash());
        assert_eq!(by_side.slot_number, Some(9));

        let by_canonical = BlockIdentifierOrHash::Hash(canonical.hash())
            .resolve_block_header(&storage)
            .await
            .expect("resolve canonical")
            .expect("canonical header");
        assert_eq!(by_canonical.slot_number, Some(4));

        let by_number = BlockIdentifierOrHash::Identifier(BlockIdentifier::Number(7))
            .resolve_block_header(&storage)
            .await
            .expect("resolve number")
            .expect("number header");
        assert_eq!(by_number.hash(), canonical.hash());
        assert_eq!(by_number.slot_number, Some(4));
    }
}
