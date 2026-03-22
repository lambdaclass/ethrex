use serde_json::Value;
use tracing::debug;

use crate::rpc::{RpcApiContext, RpcHandler};
use crate::types::account_proof::{AccountProof, StorageProof};
use crate::types::block_identifier::{BlockIdentifierOrHash, BlockTag};
use crate::utils::RpcErr;
use ethrex_common::{Address, BigEndianHash, H256, U256, serde_utils};

pub struct GetBalanceRequest {
    pub address: Address,
    pub block: BlockIdentifierOrHash,
}

pub struct GetCodeRequest {
    pub address: Address,
    pub block: BlockIdentifierOrHash,
}

pub struct GetStorageAtRequest {
    pub address: Address,
    pub storage_slot: H256,
    pub block: BlockIdentifierOrHash,
}

pub struct GetTransactionCountRequest {
    pub address: Address,
    pub block: BlockIdentifierOrHash,
}

pub struct GetProofRequest {
    pub address: Address,
    pub storage_keys: Vec<H256>,
    pub block: BlockIdentifierOrHash,
}

impl RpcHandler for GetBalanceRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetBalanceRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 2 params".to_owned()));
        };
        Ok(GetBalanceRequest {
            address: serde_json::from_value(params[0].clone())?,
            block: BlockIdentifierOrHash::parse(params[1].clone(), 1)?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested balance of account {} at block {}",
            self.address, self.block
        );

        let Some(block_number) = self.block.resolve_block_number(&context.storage).await? else {
            return Err(RpcErr::Internal(
                "Could not resolve block number".to_owned(),
            ));
        };
        let balance = context
            .storage
            .get_account_info(block_number, self.address)
            .await?
            .map(|info| info.balance)
            .unwrap_or_default();

        serde_json::to_value(format!("{balance:#x}"))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetCodeRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetCodeRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 2 params".to_owned()));
        };
        Ok(GetCodeRequest {
            address: serde_json::from_value(params[0].clone())?,
            block: BlockIdentifierOrHash::parse(params[1].clone(), 1)?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested code of account {} at block {}",
            self.address, self.block
        );

        let Some(block_number) = self.block.resolve_block_number(&context.storage).await? else {
            return Err(RpcErr::Internal(
                "Could not resolve block number".to_owned(),
            ));
        };
        let code = context
            .storage
            .get_code_by_account_address(block_number, self.address)
            .await?
            .map(|c| c.bytecode)
            .unwrap_or_default();

        serde_json::to_value(format!("0x{code:x}"))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetStorageAtRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetStorageAtRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 3 {
            return Err(RpcErr::BadParams("Expected 3 params".to_owned()));
        };
        let storage_slot_u256 = serde_utils::u256::deser_hex_or_dec_str(params[1].clone())?;
        Ok(GetStorageAtRequest {
            address: serde_json::from_value(params[0].clone())?,
            storage_slot: H256::from_uint(&storage_slot_u256),
            block: BlockIdentifierOrHash::parse(params[2].clone(), 2)?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested storage slot {} of account {} at block {}",
            self.storage_slot, self.address, self.block
        );

        let Some(block_number) = self.block.resolve_block_number(&context.storage).await? else {
            return Err(RpcErr::Internal(
                "Could not resolve block number".to_owned(),
            ));
        };
        let storage_value = context
            .storage
            .get_storage_at(block_number, self.address, self.storage_slot)?
            .unwrap_or_default();
        let storage_value = H256::from_uint(&storage_value);
        serde_json::to_value(format!("{storage_value:#x}"))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetTransactionCountRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetTransactionCountRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 2 params".to_owned()));
        };
        Ok(GetTransactionCountRequest {
            address: serde_json::from_value(params[0].clone())?,
            block: BlockIdentifierOrHash::parse(params[1].clone(), 1)?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested nonce of account {} at block {}",
            self.address, self.block
        );

        // If the tag is Pending, we need to get the nonce from the mempool
        let pending_nonce = if self.block == BlockTag::Pending {
            context.blockchain.mempool.get_nonce(&self.address)?
        } else {
            None
        };

        let nonce = match pending_nonce {
            Some(nonce) => nonce,
            None => {
                let Some(block_number) = self.block.resolve_block_number(&context.storage).await?
                else {
                    return serde_json::to_value("0x0")
                        .map_err(|error| RpcErr::Internal(error.to_string()));
                };
                context
                    .storage
                    .get_nonce_by_account_address(block_number, self.address)
                    .await?
                    .unwrap_or_default()
            }
        };

        serde_json::to_value(format!("0x{nonce:x}"))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetProofRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 3 {
            return Err(RpcErr::BadParams("Expected 3 params".to_owned()));
        };
        let storage_keys: Vec<U256> = serde_json::from_value(params[1].clone())?;
        let storage_keys = storage_keys.iter().map(H256::from_uint).collect();
        Ok(GetProofRequest {
            address: serde_json::from_value(params[0].clone())?,
            storage_keys,
            block: BlockIdentifierOrHash::parse(params[2].clone(), 2)?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        debug!(
            "Requested proof for account {} at block {} with storage keys: {:?}",
            self.address, self.block, self.storage_keys
        );
        let Some(block_number) = self.block.resolve_block_number(storage).await? else {
            return Ok(Value::Null);
        };
        let Some(block_hash) = storage.get_canonical_block_hash(block_number).await? else {
            return Ok(Value::Null);
        };

        // Read account state from binary trie to populate the response fields.
        let state = context
            .blockchain
            .binary_trie_state
            .read()
            .map_err(|e| RpcErr::Internal(format!("lock error: {e}")))?;
        let account = state
            .get_account_state_at(&self.address, block_hash)
            .map_err(|e| RpcErr::Internal(format!("binary trie error: {e}")))?
            .unwrap_or_default();

        // Storage slot values.
        let storage_proof: Vec<StorageProof> = self
            .storage_keys
            .iter()
            .map(|key| {
                let value = state
                    .get_storage_slot_at(&self.address, *key, block_hash)
                    .unwrap_or(None)
                    .unwrap_or_default();
                StorageProof {
                    key: key.into_uint(),
                    // Binary trie proofs have a different format than MPT proofs.
                    // Proof generation is not yet wired into this RPC handler.
                    proof: vec![],
                    value,
                }
            })
            .collect();

        let account_proof = AccountProof {
            // Binary trie proofs use sibling hashes, not MPT-encoded nodes.
            // Proof data is not yet wired into this RPC handler.
            account_proof: vec![],
            address: self.address,
            balance: account.balance,
            code_hash: account.code_hash,
            nonce: account.nonce,
            storage_hash: account.storage_root,
            storage_proof,
        };
        serde_json::to_value(account_proof).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_storage_at_request_parse_hex_slot() {
        let params = Some(vec![
            json!("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
            // Storage slot can be provided as hex string
            json!("0x1"),
            json!("latest"),
        ]);
        let request = GetStorageAtRequest::parse(&params).unwrap();

        let expected_address = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
            .parse()
            .unwrap();
        assert_eq!(request.address, expected_address);
        assert_eq!(request.storage_slot, H256::from_uint(&U256::from(1u64)));
        assert_eq!(request.block, BlockTag::Latest);
    }

    #[test]
    fn test_get_storage_at_request_parse_number_slot() {
        let params = Some(vec![
            json!("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
            // Storage slot can be provided as number
            json!("1"),
            json!("latest"),
        ]);
        let request = GetStorageAtRequest::parse(&params).unwrap();

        let expected_address = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
            .parse()
            .unwrap();
        assert_eq!(request.address, expected_address);
        assert_eq!(request.storage_slot, H256::from_uint(&U256::from(1u64)));
        assert_eq!(request.block, BlockTag::Latest);
    }
}
