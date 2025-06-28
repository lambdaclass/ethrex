use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCACK_HASH,
    types::{AccountInfo, AccountState, block_execution_witness::ExecutionWitnessResult},
};
use ethrex_rlp::decode::RLPDecode;
use sha3::{Digest, Keccak256};

use crate::{EvmError, VmDatabase};

impl VmDatabase for ExecutionWitnessResult {
    fn get_account_info(&self, address: Address) -> Result<Option<AccountInfo>, EvmError> {
        let state_trie = self.state_trie.as_ref().ok_or(EvmError::DB(
            "ExecutionWitness: Tried to get state trie before rebuilding tries".to_string(),
        ))?;
        let state_trie_lock = state_trie
            .lock()
            .map_err(|_| EvmError::DB("Failed to lock state trie".to_string()))?;
        let hashed_address = hash_address(&address);
        let Ok(Some(encoded_state)) = state_trie_lock.get(&hashed_address) else {
            return Ok(None);
        };
        let state = AccountState::decode(&encoded_state)
            .map_err(|_| EvmError::DB("Failed to get decode account from trie".to_string()))?;

        Ok(Some(AccountInfo {
            balance: state.balance,
            code_hash: state.code_hash,
            nonce: state.nonce,
        }))
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        self.block_headers
            .get(&block_number)
            .map(|header| header.hash())
            .ok_or_else(|| {
                EvmError::DB(format!(
                    "Block hash not found for block number {block_number}"
                ))
            })
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        let storage_tries_map = self.storage_tries.as_ref().ok_or(EvmError::DB(
            "ExecutionWitness: Tried to get storage slot before rebuilding tries".to_string(),
        ))?;

        let storage_tries_lock = storage_tries_map
            .lock()
            .map_err(|_| EvmError::DB("Failed to lock storage tries".to_string()))?;

        let Some(storage_trie) = storage_tries_lock.get(&address) else {
            return Ok(None);
        };
        let hashed_key = hash_key(&key);
        if let Some(encoded_key) = storage_trie
            .get(&hashed_key)
            .map_err(|e| EvmError::DB(e.to_string()))?
        {
            U256::decode(&encoded_key)
                .map_err(|_| EvmError::DB("failed to read storage from trie".to_string()))
                .map(Some)
        } else {
            Ok(None)
        }
    }

    fn get_chain_config(&self) -> Result<ethrex_common::types::ChainConfig, EvmError> {
        Ok(self.chain_config)
    }

    fn get_account_code(&self, code_hash: H256) -> Result<bytes::Bytes, EvmError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Bytes::new());
        }
        match self.codes.get(&code_hash) {
            Some(code) => Ok(code.clone()),
            None => Err(EvmError::DB(format!(
                "Could not find code for hash {code_hash}"
            ))),
        }
    }
}

fn hash_address(address: &Address) -> Vec<u8> {
    Keccak256::new_with_prefix(address.to_fixed_bytes())
        .finalize()
        .to_vec()
}

pub fn hash_key(key: &H256) -> Vec<u8> {
    Keccak256::new_with_prefix(key.to_fixed_bytes())
        .finalize()
        .to_vec()
}
