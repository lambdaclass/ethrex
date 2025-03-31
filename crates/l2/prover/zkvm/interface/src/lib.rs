pub mod methods {
    #[cfg(any(clippy, not(feature = "risc0")))]
    pub const ZKVM_RISC0_PROGRAM_ELF: &[u8] = &[0];
    #[cfg(any(clippy, not(feature = "risc0")))]
    pub const ZKVM_RISC0_PROGRAM_ID: [u32; 8] = [0_u32; 8];
    #[cfg(all(not(clippy), feature = "risc0"))]
    include!(concat!(env!("OUT_DIR"), "/methods.rs"));

    #[cfg(all(not(clippy), feature = "sp1"))]
    pub const ZKVM_SP1_PROGRAM_ELF: &[u8] =
        include_bytes!("../sp1/elf/riscv32im-succinct-zkvm-elf");
    #[cfg(any(clippy, not(feature = "sp1")))]
    pub const ZKVM_SP1_PROGRAM_ELF: &[u8] = &[0];

    #[cfg(all(not(clippy), feature = "pico"))]
    pub const ZKVM_PICO_PROGRAM_ELF: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/riscv32im-pico-zkvm-elf"));
    #[cfg(any(clippy, not(feature = "pico")))]
    pub const ZKVM_PICO_PROGRAM_ELF: &[u8] = &[0];
}

pub mod io {
    use ethrex_common::{
        types::{Block, BlockHeader},
        H256,
    };
    use ethrex_vm::ExecutionDB;
    use serde::{de::DeserializeOwned, Deserialize, Serialize};
    use serde_with::{serde_as, DeserializeAs, SerializeAs};

    /// Private input variables passed into the zkVM execution program.
    #[serde_as]
    #[derive(Serialize, Deserialize)]
    pub struct ProgramInput {
        /// block to execute
        #[serde_as(as = "SerdeJSON")]
        pub block: Block,
        /// header of the previous block
        #[serde_as(as = "SerdeJSON")]
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
        #[cfg(feature = "l2")]
        /// hash of all the deposit logs made in executed block
        pub deposit_logs_hash: H256,
    }

    impl ProgramOutput {
        pub fn encode(&self) -> Vec<u8> {
            [
                self.initial_state_hash.to_fixed_bytes(),
                self.final_state_hash.to_fixed_bytes(),
            ]
            .concat()
        }
    }

    /// Used with [serde_with] to encode a fields into JSON before serializing its bytes. This is
    /// necessary because a [BlockHeader] isn't compatible with other encoding formats like bincode or RLP.
    pub struct SerdeJSON;

    impl<T: Serialize> SerializeAs<T> for SerdeJSON {
        fn serialize_as<S>(val: &T, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut encoded = Vec::new();
            serde_json::to_writer(&mut encoded, val).map_err(serde::ser::Error::custom)?;
            serde_with::Bytes::serialize_as(&encoded, serializer)
        }
    }

    impl<'de, T: DeserializeOwned> DeserializeAs<'de, T> for SerdeJSON {
        fn deserialize_as<D>(deserializer: D) -> Result<T, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let encoded: Vec<u8> = serde_with::Bytes::deserialize_as(deserializer)?;
            serde_json::from_reader(&encoded[..]).map_err(serde::de::Error::custom)
        }
    }
}

pub mod trie {
    use std::collections::HashMap;

    use ethrex_common::{
        types::{AccountInfo, AccountState},
        H160, U256,
    };
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
    use ethrex_storage::{hash_address, hash_key, AccountUpdate};
    use ethrex_trie::{Trie, TrieError};
    use ethrex_vm::ExecutionDB;
    use thiserror::Error;

    #[derive(Debug, Error)]
    pub enum Error {
        #[error(transparent)]
        TrieError(#[from] TrieError),
        #[error(transparent)]
        RLPDecode(#[from] RLPDecodeError),
        #[error("Missing storage trie for address {0}")]
        MissingStorageTrie(H160),
        #[error("Missing storage for address {0}")]
        StorageNotFound(H160),
    }

    pub fn verify_db(
        db: &ExecutionDB,
        state_trie: &Trie,
        storage_tries: &HashMap<H160, Trie>,
    ) -> Result<bool, Error> {
        // verifies that, for each stored account:
        //  1. account is in state trie
        //  2. account info (nonce, balance, code hash) is correct (the same as encoded in trie)
        //  3. if there's any storage:
        //      3.a. storage root is correct (the same as encoded in trie)
        //      3.b. for each storage value:
        //          3.b.1. every value is in the storage trie, except for zero values which are absent
        //          3.b.2. every value in trie is correct (the same as encoded in trie)
        for (address, db_account_info) in &db.accounts {
            // 1. account is in state trie
            let trie_account_state = match state_trie.get(&hash_address(address)) {
                Ok(Some(encoded_state)) => AccountState::decode(&encoded_state)?,
                Ok(None) => {
                    return Ok(false);
                }
                Err(TrieError::InconsistentTree) => {
                    return Ok(false);
                }
                Err(err) => return Err(err.into()),
            };
            let trie_account_info = AccountInfo {
                nonce: trie_account_state.nonce,
                balance: trie_account_state.balance,
                code_hash: trie_account_state.code_hash,
            };

            // 2. account info is correct
            if db_account_info != &trie_account_info {
                return Ok(false);
            }

            // 3. if there's any storage
            match db.storage.get(address) {
                Some(storage) if !storage.is_empty() => {
                    let storage_trie = storage_tries
                        .get(address)
                        .ok_or(Error::MissingStorageTrie(*address))?;
                    let storage_root = storage_trie.hash_no_commit();

                    // 3.a. storage root is correct
                    if storage_root != trie_account_state.storage_root {
                        return Ok(false);
                    }

                    for (key, db_value) in storage {
                        // 3.b. every value is in storage trie, except for zero values which are
                        //      absent
                        let trie_value = match storage_trie.get(&hash_key(key)) {
                            Ok(Some(encoded)) => U256::decode(&encoded)?,
                            Ok(None) if db_value.is_zero() => {
                                // an absent value must be zero
                                continue;
                            }
                            Ok(None) | Err(TrieError::InconsistentTree) => {
                                // a non-zero value must be encoded in the trie
                                return Ok(false);
                            }
                            Err(err) => return Err(err.into()),
                        };

                        // 3.c. every value is correct
                        if *db_value != trie_value {
                            return Ok(false);
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(true)
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
                // so a proof of exclusion might not be included in the pruned state trie.
                let (mut account_state, is_account_new) = match account_state {
                    Ok(Some(encoded_state)) => (AccountState::decode(&encoded_state)?, false),
                    Ok(None) | Err(TrieError::InconsistentTree) => (AccountState::default(), true),
                    Err(err) => return Err(err.into()),
                };

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
                            .ok_or(Error::MissingStorageTrie(update.address))?
                    };
                    for (storage_key, storage_value) in &update.added_storage {
                        let hashed_key = hash_key(storage_key);
                        if storage_value.is_zero() {
                            storage_trie.remove(hashed_key)?;
                        } else {
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

#[cfg(feature = "l2")]
pub mod deposits {
    // This module was based on the L1 committer.
    // TODO: We should move this to some kind of "common" library for the L2, but the zkvm programs
    // can't depend on ethrex-l2 because of incompatible dependencies.

    use ethrex_common::{
        types::{Block, PrivilegedL2Transaction, Transaction},
        Address, U256,
    };
    use keccak_hash::{keccak, H256};

    #[derive(Debug, thiserror::Error)]
    pub enum DepositError {
        #[error("Failed to decode deposit hash")]
        FailedToDecodeHash,
        #[error("Length does not fit in u16")]
        LengthTooLarge(#[from] std::num::TryFromIntError),
    }

    #[derive(Clone)]
    pub struct DepositLog {
        pub address: Address,
        pub amount: U256,
        pub nonce: u64,
    }

    pub fn get_block_deposits(block: &Block) -> Vec<PrivilegedL2Transaction> {
        let deposits = block
            .body
            .transactions
            .iter()
            .filter_map(|tx| match tx {
                Transaction::PrivilegedL2Transaction(tx) => Some(tx.clone()),
                _ => None,
            })
            .collect();

        deposits
    }

    pub fn get_deposit_hash(deposit_hashes: Vec<H256>) -> Result<H256, DepositError> {
        if !deposit_hashes.is_empty() {
            let deposit_hashes_len: u16 = deposit_hashes
                .len()
                .try_into()
                .map_err(DepositError::from)?;
            Ok(H256::from_slice(
                [
                    &deposit_hashes_len.to_be_bytes(),
                    keccak(
                        deposit_hashes
                            .iter()
                            .map(H256::as_bytes)
                            .collect::<Vec<&[u8]>>()
                            .concat(),
                    )
                    .as_bytes()
                    .get(2..32)
                    .ok_or(DepositError::FailedToDecodeHash)?,
                ]
                .concat()
                .as_slice(),
            ))
        } else {
            Ok(H256::zero())
        }
    }
}
