use std::{fmt::Display, str::FromStr};

use ethrex_common::types::{BlockHash, BlockHeader, BlockNumber};
use ethrex_storage::{Store, error::StoreError};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::utils::RpcErr;

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
                BlockTag::Latest => Ok(Some(storage.get_latest_block_number().await?)),
                BlockTag::Pending => {
                    // TODO(#1112): We need to check individual intrincacies of the pending tag for
                    // each RPC method that uses it.
                    if let Some(pending_block_number) = storage.get_pending_block_number().await? {
                        Ok(Some(pending_block_number))
                    } else {
                        // If there are no pending blocks, we return the latest block number
                        Ok(Some(storage.get_latest_block_number().await?))
                    }
                }
            },
        }
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
    pub async fn resolve_block_header(
        &self,
        storage: &Store,
    ) -> Result<Option<BlockHeader>, StoreError> {
        match self.resolve_block_number(storage).await? {
            Some(block_number) => storage.get_block_header(block_number),
            _ => Ok(None),
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

    pub fn parse(serde_value: Value, arg_index: u64) -> Result<BlockIdentifierOrHash, RpcErr> {
        // EIP-1898 object form: {"blockHash": "0x..." [, "requireCanonical": bool]}
        // or {"blockNumber": "0x..."}.
        // `requireCanonical` is accepted for compatibility but currently ignored:
        // ethrex resolves block hashes through the canonical index, so non-canonical
        // hashes already fail to resolve.
        if let Value::Object(map) = &serde_value {
            if let Some(hash_value) = map.get("blockHash") {
                let hex_str: String = serde_json::from_value(hash_value.clone())
                    .map_err(|err| RpcErr::BadParams(err.to_string()))?;
                let block_hash = BlockHash::from_str(&hex_str)
                    .map_err(|_| RpcErr::BadHexFormat(arg_index))?;
                return Ok(BlockIdentifierOrHash::Hash(block_hash));
            }
            if let Some(number_value) = map.get("blockNumber") {
                return BlockIdentifier::parse(number_value.clone(), arg_index)
                    .map(BlockIdentifierOrHash::Identifier);
            }
            return Err(RpcErr::BadParams(
                "expected blockHash or blockNumber field".to_owned(),
            ));
        }

        // String form: a 32-byte block hash, a hex block number, or a block tag.
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
        let latest = storage.get_latest_block_number().await?;

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

    const HASH_HEX: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn parse_string_block_hash() {
        let parsed = BlockIdentifierOrHash::parse(json!(HASH_HEX), 0).unwrap();
        match parsed {
            BlockIdentifierOrHash::Hash(hash) => {
                assert_eq!(hash, BlockHash::from_str(HASH_HEX).unwrap());
            }
            _ => panic!("expected Hash variant"),
        }
    }

    #[test]
    fn parse_string_block_number() {
        let parsed = BlockIdentifierOrHash::parse(json!("0x10"), 0).unwrap();
        assert!(matches!(
            parsed,
            BlockIdentifierOrHash::Identifier(BlockIdentifier::Number(16))
        ));
    }

    #[test]
    fn parse_string_block_tag() {
        let parsed = BlockIdentifierOrHash::parse(json!("latest"), 0).unwrap();
        assert!(matches!(
            parsed,
            BlockIdentifierOrHash::Identifier(BlockIdentifier::Tag(BlockTag::Latest))
        ));
    }

    // EIP-1898: {"blockHash": "0x..."} — used by go-ethereum's ethclient when
    // calling eth_getBlockReceipts with rpc.BlockNumberOrHash{BlockHash: ...}.
    #[test]
    fn parse_eip1898_block_hash_object() {
        let parsed = BlockIdentifierOrHash::parse(json!({ "blockHash": HASH_HEX }), 0).unwrap();
        match parsed {
            BlockIdentifierOrHash::Hash(hash) => {
                assert_eq!(hash, BlockHash::from_str(HASH_HEX).unwrap());
            }
            _ => panic!("expected Hash variant"),
        }
    }

    #[test]
    fn parse_eip1898_block_hash_object_with_require_canonical() {
        let parsed = BlockIdentifierOrHash::parse(
            json!({ "blockHash": HASH_HEX, "requireCanonical": true }),
            0,
        )
        .unwrap();
        assert!(matches!(parsed, BlockIdentifierOrHash::Hash(_)));
    }

    // EIP-1898: {"blockNumber": "0x..."}
    #[test]
    fn parse_eip1898_block_number_object() {
        let parsed = BlockIdentifierOrHash::parse(json!({ "blockNumber": "0x2a" }), 0).unwrap();
        assert!(matches!(
            parsed,
            BlockIdentifierOrHash::Identifier(BlockIdentifier::Number(42))
        ));
    }

    #[test]
    fn parse_eip1898_empty_object_errors() {
        let result = BlockIdentifierOrHash::parse(json!({}), 0);
        assert!(matches!(result, Err(RpcErr::BadParams(_))));
    }

    #[test]
    fn parse_eip1898_bad_hash_errors() {
        let result = BlockIdentifierOrHash::parse(json!({ "blockHash": "not-a-hash" }), 0);
        assert!(matches!(result, Err(RpcErr::BadHexFormat(_))));
    }
}
