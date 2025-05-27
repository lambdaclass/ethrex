pub mod methods {
    #[cfg(any(clippy, not(feature = "risc0")))]
    pub const ZKVM_RISC0_PROGRAM_ELF: &[u8] = &[0];
    #[cfg(any(clippy, not(feature = "risc0")))]
    pub const ZKVM_RISC0_PROGRAM_ID: [u32; 8] = [0_u32; 8];
    #[cfg(all(not(clippy), feature = "risc0"))]
    include!(concat!(env!("OUT_DIR"), "/methods.rs"));

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
    use ethrex_vm::ProverDB;
    use serde::{de::DeserializeOwned, Deserialize, Serialize};
    use serde_with::{serde_as, DeserializeAs, SerializeAs};

    /// Private input variables passed into the zkVM execution program.
    #[serde_as]
    #[derive(Serialize, Deserialize)]
    pub struct ProgramInput {
        /// blocks to execute
        #[serde_as(as = "SerdeJSON")]
        pub blocks: Vec<Block>,
        /// header of the previous block
        #[serde_as(as = "SerdeJSON")]
        pub parent_block_header: BlockHeader,
        /// database containing only the data necessary to execute
        pub db: ProverDB,
        /// value used to calculate base fee
        pub elasticity_multiplier: u64,
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
        /// merkle root of all withdrawals in a batch
        pub withdrawals_merkle_root: H256,
        #[cfg(feature = "l2")]
        /// hash of all the deposit logs made in a batch
        pub deposit_logs_hash: H256,
    }

    impl ProgramOutput {
        pub fn encode(&self) -> Vec<u8> {
            [
                self.initial_state_hash.to_fixed_bytes(),
                self.final_state_hash.to_fixed_bytes(),
                #[cfg(feature = "l2")]
                self.withdrawals_merkle_root.to_fixed_bytes(),
                #[cfg(feature = "l2")]
                self.deposit_logs_hash.to_fixed_bytes(),
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
            let trie_account_state = match state_trie.get(hash_address(address)) {
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
                        let trie_value = match storage_trie.get(hash_key(key)) {
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

pub mod withdrawals {
    // TODO: We should move this to some kind of "common" library for the L2, but the zkvm programs
    // can't depend on ethrex-l2 because of incompatible dependencies.

    use std::str::FromStr;

    use ethrex_common::{
        types::{Receipt, Transaction, TxKind},
        Address, H160, H256,
    };

    use keccak_hash::keccak;
    use thiserror::Error;

    pub const COMMON_BRIDGE_L2_ADDRESS: Address = H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0xff, 0xff,
    ]);

    #[derive(Debug, Error)]
    pub enum Error {
        #[error("Withdrawal transaction was invalid")]
        InvalidWithdrawalTransaction,
        #[error("Failed to merkelize withdrawals")]
        FailedToMerkelize,
        #[error("Failed to create withdrawal selector")]
        WithdrawalSelector,
        #[error("Failed to get withdrawal hash")]
        WithdrawalHash,
    }

    pub fn get_block_withdrawals(
        txs: &[Transaction],
        receipts: &[Receipt],
    ) -> Result<Vec<H256>, Error> {
        txs.iter()
            .zip(receipts.iter())
            .filter(|(tx, receipt)| is_withdrawal_l2(tx, receipt))
            .map(|(withdrawal, _)| get_withdrawal_hash(withdrawal).ok_or(Error::WithdrawalHash))
            .collect::<Result<Vec<_>, _>>()
    }

    fn is_withdrawal_l2(tx: &Transaction, receipt: &Receipt) -> bool {
        // WithdrawalInitiated(address,address,uint256)
        let withdrawal_event_selector: H256 =
            H256::from_str("bb2689ff876f7ef453cf8865dde5ab10349d222e2e1383c5152fbdb083f02da2")
                .unwrap();

        match tx.to() {
            TxKind::Call(to) if to == COMMON_BRIDGE_L2_ADDRESS => receipt.logs.iter().any(|log| {
                log.topics
                    .iter()
                    .any(|topic| *topic == withdrawal_event_selector)
            }),
            _ => false,
        }
    }

    pub fn get_withdrawals_merkle_root(withdrawals_hashes: Vec<H256>) -> Result<H256, Error> {
        if !withdrawals_hashes.is_empty() {
            merkelize(withdrawals_hashes)
        } else {
            Ok(H256::zero())
        }
    }

    pub fn get_withdrawal_hash(tx: &Transaction) -> Option<H256> {
        let to_bytes: [u8; 20] = match tx.data().get(16..36)?.try_into() {
            Ok(value) => value,
            Err(_) => return None,
        };
        let to = Address::from(to_bytes);

        let value = tx.value().to_big_endian();

        Some(keccak_hash::keccak(
            [to.as_bytes(), &value, tx.compute_hash().as_bytes()].concat(),
        ))
    }

    pub fn merkelize(data: Vec<H256>) -> Result<H256, Error> {
        let mut data = data;
        let mut first = true;
        while data.len() > 1 || first {
            first = false;
            data = data
                .chunks(2)
                .flat_map(|chunk| -> Result<H256, Error> {
                    let left = chunk.first().ok_or(Error::FailedToMerkelize)?;
                    let right = *chunk.get(1).unwrap_or(left);
                    Ok(keccak([left.as_bytes(), right.as_bytes()].concat())
                        .as_fixed_bytes()
                        .into())
                })
                .collect();
        }
        data.first().copied().ok_or(Error::FailedToMerkelize)
    }
}

#[cfg(feature = "l2")]
pub mod deposits {
    // This module was based on the L1 committer.
    // TODO: We should move this to some kind of "common" library for the L2, but the zkvm programs
    // can't depend on ethrex-l2 because of incompatible dependencies.

    use ethrex_common::{
        types::{PrivilegedL2Transaction, Transaction},
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

    pub fn get_block_deposits(txs: &[Transaction]) -> Vec<PrivilegedL2Transaction> {
        txs.iter()
            .filter_map(|tx| match tx {
                Transaction::PrivilegedL2Transaction(tx) => Some(tx.clone()),
                _ => None,
            })
            .collect()
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
