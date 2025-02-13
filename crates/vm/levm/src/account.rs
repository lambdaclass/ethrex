use crate::constants::EMPTY_CODE_HASH;
use bytes::Bytes;
use ethrex_common::{H256, U256};
use keccak_hash::keccak;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountInfo {
    pub balance: U256,
    pub bytecode: Bytes,
    pub nonce: u64,
}

impl AccountInfo {
    pub fn is_empty(&self) -> bool {
        self.balance.is_zero() && self.nonce == 0 && self.bytecode.is_empty()
    }

    pub fn has_code(&self) -> bool {
        !(self.bytecode.is_empty() || self.bytecode_hash() == EMPTY_CODE_HASH)
    }

    pub fn bytecode_hash(&self) -> H256 {
        keccak(self.bytecode.as_ref()).0.into()
    }

    pub fn has_nonce(&self) -> bool {
        self.nonce != 0
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub info: AccountInfo,
    pub storage: HashMap<H256, StorageSlot>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageSlot {
    pub original_value: U256,
    pub current_value: U256,
}

impl From<AccountInfo> for Account {
    fn from(info: AccountInfo) -> Self {
        Self {
            info,
            storage: HashMap::new(),
        }
    }
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

    pub fn has_nonce(&self) -> bool {
        self.info.has_nonce()
    }

    pub fn has_code(&self) -> bool {
        self.info.has_code()
    }

    pub fn has_code_or_nonce(&self) -> bool {
        self.has_code() || self.has_nonce()
    }

    pub fn bytecode_hash(&self) -> H256 {
        self.info.bytecode_hash()
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
}
