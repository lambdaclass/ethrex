use std::collections::HashMap;

use ethrex_common::{
    types::{AccountInfo, AccountState, AccountUpdate},
    H160, U256,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use ethrex_storage::{hash_address, hash_key};
use ethrex_trie::{Trie, TrieError};
use ethrex_vm::ProverDB;
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
    db: &ProverDB,
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
