use ethrex_blockchain::mempool;
use serde_json::Value;
use tracing::info;

use crate::types::account_proof::{AccountProof, StorageProof};
use crate::types::block_identifier::{BlockIdentifierOrHash, BlockTag};
use crate::RpcApiContext;
use crate::{utils::RpcErr, RpcHandler};
use ethrex_core::{Address, BigEndianHash, H256, U256};

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
    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        info!(
            "Requested balance of account {} at block {}",
            self.address, self.block
        );

        let Some(block_number) = self.block.resolve_block_number(&context.storage)? else {
            return Err(RpcErr::Internal(
                "Could not resolve block number".to_owned(),
            )); // Should we return Null here?
        };

        let account = context
            .storage
            .get_account_info(block_number, self.address)?;
        let balance = account.map(|acc| acc.balance).unwrap_or_default();

        serde_json::to_value(format!("{:#x}", balance))
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
    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        info!(
            "Requested code of account {} at block {}",
            self.address, self.block
        );

        let Some(block_number) = self.block.resolve_block_number(&context.storage)? else {
            return Err(RpcErr::Internal(
                "Could not resolve block number".to_owned(),
            )); // Should we return Null here?
        };

        let code = context
            .storage
            .get_code_by_account_address(block_number, self.address)?
            .unwrap_or_default();

        serde_json::to_value(format!("0x{:x}", code))
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
        Ok(GetStorageAtRequest {
            address: serde_json::from_value(params[0].clone())?,
            storage_slot: serde_json::from_value(params[1].clone())?,
            block: BlockIdentifierOrHash::parse(params[2].clone(), 2)?,
        })
    }
    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        info!(
            "Requested storage sot {} of account {} at block {}",
            self.storage_slot, self.address, self.block
        );

        let Some(block_number) = self.block.resolve_block_number(&context.storage)? else {
            return Err(RpcErr::Internal(
                "Could not resolve block number".to_owned(),
            )); // Should we return Null here?
        };

        let storage_value = context
            .storage
            .get_storage_at(block_number, self.address, self.storage_slot)?
            .unwrap_or_default();
        let storage_value = H256::from_uint(&storage_value);
        serde_json::to_value(format!("{:#x}", storage_value))
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
    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        info!(
            "Requested nonce of account {} at block {}",
            self.address, self.block
        );

        // If the tag is Pending, we need to get the nonce from the mempool
        let pending_nonce = if self.block == BlockTag::Pending {
            mempool::get_nonce(&self.address, &context.storage)?
        } else {
            None
        };

        let nonce = match pending_nonce {
            Some(nonce) => nonce,
            None => {
                let Some(block_number) = self.block.resolve_block_number(&context.storage)? else {
                    return serde_json::to_value("0x0")
                        .map_err(|error| RpcErr::Internal(error.to_string()));
                };

                context
                    .storage
                    .get_nonce_by_account_address(block_number, self.address)?
                    .unwrap_or_default()
            }
        };

        serde_json::to_value(format!("0x{:x}", nonce))
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

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        info!(
            "Requested proof for account {} at block {} with storage keys: {:?}",
            self.address, self.block, self.storage_keys
        );
        let Some(block_number) = self.block.resolve_block_number(storage)? else {
            return Ok(Value::Null);
        };
        // Create account proof
        let Some(account) = storage.get_account_state(block_number, self.address)? else {
            return Ok(Value::Null);
        };
        let Some(account_proof) = storage.get_account_proof(block_number, &self.address)? else {
            return Err(RpcErr::Internal("Could not get account proof".to_owned()));
        };
        // Create storage proofs for all provided storage keys
        let mut storage_proofs = Vec::new();
        for storage_key in self.storage_keys.iter() {
            let value = storage
                .get_storage_at(block_number, self.address, *storage_key)?
                .unwrap_or_default();
            let proof =
                storage.get_storage_proof(self.address, account.storage_root, storage_key)?;
            let storage_proof = StorageProof {
                key: storage_key.into_uint(),
                proof,
                value,
            };
            storage_proofs.push(storage_proof);
        }
        let account_proof = AccountProof {
            account_proof,
            address: self.address,
            balance: account.balance,
            code_hash: account.code_hash,
            nonce: account.nonce,
            storage_hash: account.storage_root,
            storage_proof: storage_proofs,
        };
        serde_json::to_value(account_proof).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
