use std::{collections::HashMap, str::FromStr};

use bytes::Bytes;
use ethereum_rust_core::{H256, U256};
use keccak_hash::keccak;

use crate::constants::EMPTY_CODE_HASH_STR;

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct AccountInfo {
    pub balance: U256,
    pub bytecode: Bytes,
    pub nonce: u64,
}

impl AccountInfo {
    pub fn is_empty(&self) -> bool {
        self.balance.is_zero() && self.nonce == 0 && self.bytecode.is_empty()
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Account {
    pub info: AccountInfo,
    pub storage: HashMap<H256, StorageSlot>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageSlot {
    pub original_value: U256,
    pub current_value: U256,
}

impl Account {
    pub fn new(
        balance: U256,
        bytecode: Bytes,
        nonce: u64,
        storage: HashMap<H256, StorageSlot>,
    ) -> Self {
        Self {
            info: AccountInfo {
                balance,
                bytecode,
                nonce,
            },
            storage,
        }
    }

    pub fn has_code(&self) -> bool {
        !(self.info.bytecode.is_empty()
            || self.bytecode_hash() == H256::from_str(EMPTY_CODE_HASH_STR).unwrap())
    }

    pub fn bytecode_hash(&self) -> H256 {
        keccak(self.info.bytecode.as_ref()).0.into()
    }

    pub fn is_empty(&self) -> bool {
        self.info.balance.is_zero() && self.info.nonce == 0 && self.info.bytecode.is_empty()
    }

    pub fn with_balance(mut self, balance: U256) -> Self {
        self.info.balance = balance;
        self
    }

    pub fn with_bytecode(mut self, bytecode: Bytes) -> Self {
        self.info.bytecode = bytecode;
        self
    }

    pub fn with_storage(mut self, storage: HashMap<H256, StorageSlot>) -> Self {
        self.storage = storage;
        self
    }

    pub fn with_nonce(mut self, nonce: u64) -> Self {
        self.info.nonce = nonce;
        self
    }

    pub fn increment_nonce(&mut self) {
        self.info.nonce += 1;
    }
}
