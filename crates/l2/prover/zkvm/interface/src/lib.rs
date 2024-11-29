pub mod methods {
    #[cfg(any(clippy, not(feature = "build_zkvm")))]
    pub const ZKVM_PROGRAM_ELF: &[u8] = &[0];
    #[cfg(any(clippy, not(feature = "build_zkvm")))]
    pub const ZKVM_PROGRAM_ID: [u32; 8] = [0_u32; 8];

    #[cfg(all(not(clippy), feature = "build_zkvm"))]
    include!(concat!(env!("OUT_DIR"), "/methods.rs"));
}

pub mod io {
    use ethrex_core::{
        types::{Block, BlockHeader},
        H256,
    };
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
    use ethrex_vm::execution_db::ExecutionDB;
    use serde::{Deserialize, Serialize};
    use serde_with::{serde_as, DeserializeAs, SerializeAs};

    /// Private input variables passed into the zkVM execution program.
    #[serde_as]
    #[derive(Serialize, Deserialize)]
    pub struct ProgramInput {
        /// block to execute
        #[serde_as(as = "RLPBlock")]
        pub block: Block,
        /// header of the previous block
        pub parent_block_header: BlockHeader,
        /// database containing only the data necessary to execute
        pub db: ExecutionDB,
    }

    /// Public output variables exposed by the zkVM execution program. Some of these are part of
    /// the program input.
    #[derive(Serialize, Deserialize)]
    pub struct ProgramOutput {
        /// initial state trie root hash
        pub initial_state_hash: H256,
        /// final state trie root hash
        pub final_state_hash: H256,
    }

    /// Used with [serde_with] to encode a Block into RLP before serializing its bytes. This is
    /// necessary because the [ethrex_core::types::Transaction] type doesn't serializes into any
    /// format other than JSON.
    pub struct RLPBlock;

    impl SerializeAs<Block> for RLPBlock {
        fn serialize_as<S>(val: &Block, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut encoded = Vec::new();
            val.encode(&mut encoded);
            serde_with::Bytes::serialize_as(&encoded, serializer)
        }
    }

    impl<'de> DeserializeAs<'de, Block> for RLPBlock {
        fn deserialize_as<D>(deserializer: D) -> Result<Block, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let encoded: Vec<u8> = serde_with::Bytes::deserialize_as(deserializer)?;
            Block::decode(&encoded).map_err(serde::de::Error::custom)
        }
    }
}

pub mod trie {
    use std::collections::HashMap;

    use ethrex_core::{types::AccountState, H160};
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
    use ethrex_storage::{hash_address, hash_key, AccountUpdate};
    use ethrex_trie::{Trie, TrieError};
    use thiserror::Error;

    #[derive(Debug, Error)]
    pub enum Error {
        #[error(transparent)]
        TrieError(#[from] TrieError),
        #[error(transparent)]
        RLPDecode(#[from] RLPDecodeError),
        #[error("Missing storage trie for address {0}")]
        StorageNotFound(H160),
    }

    pub fn update_tries(
        state_trie: &mut Trie,
        storage_tries: &mut HashMap<H160, Trie>,
        account_updates: &[AccountUpdate],
    ) -> Result<(), Error> {
        for update in account_updates.iter() {
            let hashed_address = hash_address(&update.address);
            if update.removed {
                // Remove account from trie
                state_trie.remove(hashed_address)?;
            } else {
                // Add or update AccountState in the trie
                // Fetch current state or create a new state to be inserted
                let account_state = state_trie.get(&hashed_address);

                // if there isn't a path into the account (inconsistent tree error), then
                // it's potentially a new account. This is because we're using pruned tries
                // so the path into a new account might not be included in the pruned state trie.
                let mut account_state = match account_state {
                    Ok(Some(encoded_state)) => AccountState::decode(&encoded_state)?,
                    Ok(None) | Err(TrieError::InconsistentTree) => AccountState::default(),
                    Err(err) => return Err(err.into()),
                };
                let is_account_new = account_state == AccountState::default();

                if let Some(info) = &update.info {
                    account_state.nonce = info.nonce;
                    account_state.balance = info.balance;
                    account_state.code_hash = info.code_hash;
                }
                // Store the added storage in the account's storage trie and compute its new root
                if !update.added_storage.is_empty() {
                    let storage_trie = if is_account_new {
                        let trie = Trie::from_nodes(None, &[])?;
                        storage_tries.insert(update.address, trie);
                        storage_tries.get_mut(&update.address).unwrap()
                    } else {
                        storage_tries
                            .get_mut(&update.address)
                            .ok_or(Error::StorageNotFound(update.address))?
                    };
                    for (storage_key, storage_value) in &update.added_storage {
                        let hashed_key = hash_key(storage_key);
                        if storage_value.is_zero() && !is_account_new {
                            storage_trie.remove(hashed_key)?;
                        } else if !storage_value.is_zero() {
                            storage_trie.insert(hashed_key, storage_value.encode_to_vec())?;
                        }
                    }
                    account_state.storage_root = storage_trie.hash_no_commit();
                }
                state_trie.insert(hashed_address, account_state.encode_to_vec())?;
            }
        }
        Ok(())
    }
}
