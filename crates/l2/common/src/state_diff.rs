use std::collections::{BTreeMap, HashMap};

use bytes::Bytes;
use ethereum_types::{Address, H256, U256};
use ethrex_common::types::{
    AccountInfo, AccountState, AccountUpdate, BlockHeader, PrivilegedL2Transaction, TxKind,
    code_hash,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{error::StoreError, hash_address};
use ethrex_trie::{Trie, TrieError};
use ethrex_vm::{EvmError, VmDatabase};
use serde::{Deserialize, Serialize};

use lazy_static::lazy_static;

use crate::{deposits::DepositLog, l1_messages::L1Message};

lazy_static! {
    /// The serialized length of a default l1message log
    pub static ref L1MESSAGE_LOG_LEN: usize = L1Message::default().encode().len();

    /// The serialized length of a default deposit log
    pub static ref DEPOSITS_LOG_LEN: usize = DepositLog::default().encode().len();

    /// The serialized lenght of a default block header
    pub static ref BLOCK_HEADER_LEN: usize = encode_block_header(&BlockHeader::default()).len();
}

// State diff size for a simple transfer.
// Two `AccountUpdates` with new_balance, one of which also has nonce_diff.
pub const SIMPLE_TX_STATE_DIFF_SIZE: usize = 116;

#[derive(Debug, thiserror::Error)]
pub enum StateDiffError {
    #[error("StateDiff failed to deserialize: {0}")]
    FailedToDeserializeStateDiff(String),
    #[error("StateDiff failed to serialize: {0}")]
    FailedToSerializeStateDiff(String),
    #[error("StateDiff invalid account state diff type: {0}")]
    InvalidAccountStateDiffType(u8),
    #[error("StateDiff unsupported version: {0}")]
    UnsupportedVersion(u8),
    #[error("Both bytecode and bytecode hash are set")]
    BytecodeAndBytecodeHashSet,
    #[error("Empty account diff")]
    EmptyAccountDiff,
    #[error("The length of the vector is too big to fit in u16: {0}")]
    LengthTooBig(#[from] core::num::TryFromIntError),
    #[error("DB Error: {0}")]
    DbError(#[from] TrieError),
    #[error("Store Error: {0}")]
    StoreError(#[from] StoreError),
    #[error("New nonce is lower than the previous one")]
    FailedToCalculateNonce,
    #[error("Unexpected Error: {0}")]
    InternalError(String),
    #[error("Evm Error: {0}")]
    EVMError(#[from] EvmError),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountStateDiff {
    pub new_balance: Option<U256>,
    pub nonce_diff: u16,
    pub storage: BTreeMap<H256, U256>,
    pub bytecode: Option<Bytes>,
    pub bytecode_hash: Option<H256>,
}

#[derive(Debug, Clone, Copy)]
pub enum AccountStateDiffType {
    NewBalance = 1,
    NonceDiff = 2,
    Storage = 4,
    Bytecode = 8,
    BytecodeHash = 16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDiff {
    pub version: u8,
    pub last_header: BlockHeader,
    pub modified_accounts: BTreeMap<Address, AccountStateDiff>,
    pub l1_messages: Vec<L1Message>,
    pub deposit_logs: Vec<DepositLog>,
}

impl TryFrom<u8> for AccountStateDiffType {
    type Error = StateDiffError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(AccountStateDiffType::NewBalance),
            2 => Ok(AccountStateDiffType::NonceDiff),
            4 => Ok(AccountStateDiffType::Storage),
            8 => Ok(AccountStateDiffType::Bytecode),
            16 => Ok(AccountStateDiffType::BytecodeHash),
            _ => Err(StateDiffError::InvalidAccountStateDiffType(value)),
        }
    }
}

impl From<AccountStateDiffType> for u8 {
    fn from(value: AccountStateDiffType) -> Self {
        match value {
            AccountStateDiffType::NewBalance => 1,
            AccountStateDiffType::NonceDiff => 2,
            AccountStateDiffType::Storage => 4,
            AccountStateDiffType::Bytecode => 8,
            AccountStateDiffType::BytecodeHash => 16,
        }
    }
}

impl AccountStateDiffType {
    // Checks if the type is present in the given value
    pub fn is_in(&self, value: u8) -> bool {
        value & u8::from(*self) == u8::from(*self)
    }
}

impl Default for StateDiff {
    fn default() -> Self {
        StateDiff {
            version: 1,
            last_header: BlockHeader::default(),
            modified_accounts: BTreeMap::new(),
            l1_messages: Vec::new(),
            deposit_logs: Vec::new(),
        }
    }
}

pub fn encode_block_header(block_header: &BlockHeader) -> Vec<u8> {
    let mut encoded = Vec::new();
    encoded.extend(block_header.transactions_root.0);
    encoded.extend(block_header.receipts_root.0);
    encoded.extend(block_header.parent_hash.0);
    encoded.extend(block_header.gas_limit.to_be_bytes());
    encoded.extend(block_header.gas_used.to_be_bytes());
    encoded.extend(block_header.timestamp.to_be_bytes());
    encoded.extend(block_header.number.to_be_bytes());
    encoded.extend(block_header.base_fee_per_gas.unwrap_or(0).to_be_bytes());

    encoded
}

impl StateDiff {
    pub fn encode(&self) -> Result<Bytes, StateDiffError> {
        if self.version != 1 {
            return Err(StateDiffError::UnsupportedVersion(self.version));
        }

        let mut encoded: Vec<u8> = Vec::new();
        encoded.push(self.version);

        let header_encoded = encode_block_header(&self.last_header);
        encoded.extend(header_encoded);

        let modified_accounts_len: u16 = self
            .modified_accounts
            .len()
            .try_into()
            .map_err(StateDiffError::from)?;
        encoded.extend(modified_accounts_len.to_be_bytes());

        for (address, diff) in &self.modified_accounts {
            let account_encoded = diff.encode(address)?;
            encoded.extend(account_encoded);
        }

        let message_len: u16 = self.l1_messages.len().try_into()?;
        encoded.extend(message_len.to_be_bytes());
        for message in self.l1_messages.iter() {
            let message_encoded = message.encode();
            encoded.extend(message_encoded);
        }

        let deposits_len: u16 = self.deposit_logs.len().try_into()?;
        encoded.extend(deposits_len.to_be_bytes());
        for deposit in self.deposit_logs.iter() {
            let deposit_encoded = deposit.encode();
            encoded.extend(deposit_encoded);
        }

        Ok(Bytes::from(encoded))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, StateDiffError> {
        let mut decoder = Decoder::new(bytes);

        let version = decoder.get_u8()?;
        if version != 0x01 {
            return Err(StateDiffError::UnsupportedVersion(version));
        }

        // Last header fields
        let last_header = BlockHeader {
            transactions_root: decoder.get_h256()?,
            receipts_root: decoder.get_h256()?,
            parent_hash: decoder.get_h256()?,
            gas_limit: decoder.get_u64()?,
            gas_used: decoder.get_u64()?,
            timestamp: decoder.get_u64()?,
            number: decoder.get_u64()?,
            base_fee_per_gas: Some(decoder.get_u64()?),
            ..Default::default()
        };

        // Accounts diff
        let modified_accounts_len = decoder.get_u16()?;

        let mut modified_accounts = BTreeMap::new();
        for _ in 0..modified_accounts_len {
            let next_bytes = bytes.get(decoder.consumed()..).ok_or(
                StateDiffError::FailedToSerializeStateDiff("Not enough bytes".to_string()),
            )?;
            let (bytes_read, address, account_diff) = AccountStateDiff::decode(next_bytes)?;
            decoder.advance(bytes_read);
            modified_accounts.insert(address, account_diff);
        }

        let l1messages_len = decoder.get_u16()?;

        let mut l1messages = Vec::with_capacity(l1messages_len.into());
        for _ in 0..l1messages_len {
            let tx = decoder.get_h256()?;
            let from = decoder.get_address()?;
            let data = decoder.get_h256()?;

            l1messages.push(L1Message {
                from,
                data_hash: data,
                tx_hash: tx,
            });
        }

        let deposit_logs_len = decoder.get_u16()?;

        let mut deposit_logs = Vec::with_capacity(deposit_logs_len.into());
        for _ in 0..deposit_logs_len {
            let address = decoder.get_address()?;
            let amount = decoder.get_u256()?;

            deposit_logs.push(DepositLog {
                address,
                amount,
                nonce: Default::default(),
            });
        }

        Ok(Self {
            version,
            last_header,
            modified_accounts,
            l1_messages: l1messages,
            deposit_logs,
        })
    }

    pub fn to_account_updates(
        &self,
        prev_state: &Trie,
    ) -> Result<HashMap<Address, AccountUpdate>, StateDiffError> {
        let mut account_updates = HashMap::new();

        for (address, diff) in &self.modified_accounts {
            let account_state = match prev_state
                .get(&hash_address(address))
                .map_err(StateDiffError::DbError)?
            {
                Some(rlp) => AccountState::decode(&rlp)
                    .map_err(|e| StateDiffError::FailedToDeserializeStateDiff(e.to_string()))?,
                None => AccountState::default(),
            };

            let balance = diff.new_balance.unwrap_or(account_state.balance);
            let nonce = account_state.nonce + u64::from(diff.nonce_diff);
            let bytecode_hash = diff.bytecode_hash.unwrap_or_else(|| match &diff.bytecode {
                Some(bytecode) => code_hash(bytecode),
                None => code_hash(&Bytes::new()),
            });

            let account_info = if diff.new_balance.is_some()
                || diff.nonce_diff != 0
                || diff.bytecode_hash.is_some()
            {
                Some(AccountInfo {
                    balance,
                    nonce,
                    code_hash: bytecode_hash,
                })
            } else {
                None
            };

            account_updates.insert(
                *address,
                AccountUpdate {
                    address: *address,
                    removed: false,
                    info: account_info,
                    code: diff.bytecode.clone(),
                    added_storage: diff.storage.clone().into_iter().collect(),
                },
            );
        }

        Ok(account_updates)
    }
}

impl AccountStateDiff {
    pub fn encode(&self, address: &Address) -> Result<Vec<u8>, StateDiffError> {
        if self.bytecode.is_some() && self.bytecode_hash.is_some() {
            return Err(StateDiffError::BytecodeAndBytecodeHashSet);
        }

        let mut r#type = 0;
        let mut encoded: Vec<u8> = Vec::new();

        if let Some(new_balance) = self.new_balance {
            let r_type: u8 = AccountStateDiffType::NewBalance.into();
            r#type += r_type;
            encoded.extend_from_slice(&new_balance.to_big_endian());
        }

        if self.nonce_diff != 0 {
            let r_type: u8 = AccountStateDiffType::NonceDiff.into();
            r#type += r_type;
            encoded.extend(self.nonce_diff.to_be_bytes());
        }

        if !self.storage.is_empty() {
            let r_type: u8 = AccountStateDiffType::Storage.into();
            let storage_len: u16 = self
                .storage
                .len()
                .try_into()
                .map_err(StateDiffError::from)?;
            r#type += r_type;
            encoded.extend(storage_len.to_be_bytes());
            for (key, value) in &self.storage {
                encoded.extend_from_slice(&key.0);
                encoded.extend_from_slice(&value.to_big_endian());
            }
        }

        if let Some(bytecode) = &self.bytecode {
            let r_type: u8 = AccountStateDiffType::Bytecode.into();
            let bytecode_len: u16 = bytecode.len().try_into().map_err(StateDiffError::from)?;
            r#type += r_type;
            encoded.extend(bytecode_len.to_be_bytes());
            encoded.extend(bytecode);
        }

        if let Some(bytecode_hash) = &self.bytecode_hash {
            let r_type: u8 = AccountStateDiffType::BytecodeHash.into();
            r#type += r_type;
            encoded.extend(&bytecode_hash.0);
        }

        if r#type == 0 {
            return Err(StateDiffError::EmptyAccountDiff);
        }

        let mut result = Vec::with_capacity(1 + address.0.len() + encoded.len());
        result.extend(r#type.to_be_bytes());
        result.extend(address.0);
        result.extend(encoded);

        Ok(result)
    }

    /// Returns a tuple of the number of bytes read, the address of the account
    /// and the decoded `AccountStateDiff`
    pub fn decode(bytes: &[u8]) -> Result<(usize, Address, Self), StateDiffError> {
        let mut decoder = Decoder::new(bytes);

        let update_type = decoder.get_u8()?;

        let address = decoder.get_address()?;

        let new_balance = if AccountStateDiffType::NewBalance.is_in(update_type) {
            Some(decoder.get_u256()?)
        } else {
            None
        };

        let nonce_diff = if AccountStateDiffType::NonceDiff.is_in(update_type) {
            Some(decoder.get_u16()?)
        } else {
            None
        };

        let mut storage_diff = BTreeMap::new();
        if AccountStateDiffType::Storage.is_in(update_type) {
            let storage_slots_updated = decoder.get_u16()?;

            for _ in 0..storage_slots_updated {
                let key = decoder.get_h256()?;
                let new_value = decoder.get_u256()?;

                storage_diff.insert(key, new_value);
            }
        }

        let bytecode = if AccountStateDiffType::Bytecode.is_in(update_type) {
            let bytecode_len = decoder.get_u16()?;
            Some(decoder.get_bytes(bytecode_len.into())?)
        } else {
            None
        };

        let bytecode_hash = if AccountStateDiffType::BytecodeHash.is_in(update_type) {
            Some(decoder.get_h256()?)
        } else {
            None
        };

        Ok((
            decoder.consumed(),
            address,
            AccountStateDiff {
                new_balance,
                nonce_diff: nonce_diff.unwrap_or(0),
                storage: storage_diff,
                bytecode,
                bytecode_hash,
            },
        ))
    }
}

struct Decoder {
    bytes: Bytes,
    offset: usize,
}

impl Decoder {
    fn new(bytes: &[u8]) -> Self {
        Decoder {
            bytes: Bytes::copy_from_slice(bytes),
            offset: 0,
        }
    }

    fn consumed(&self) -> usize {
        self.offset
    }

    fn advance(&mut self, size: usize) {
        self.offset += size;
    }

    fn get_address(&mut self) -> Result<Address, StateDiffError> {
        let res = Address::from_slice(self.bytes.get(self.offset..self.offset + 20).ok_or(
            StateDiffError::FailedToDeserializeStateDiff("Not enough bytes".to_string()),
        )?);
        self.offset += 20;

        Ok(res)
    }

    fn get_u256(&mut self) -> Result<U256, StateDiffError> {
        let res = U256::from_big_endian(self.bytes.get(self.offset..self.offset + 32).ok_or(
            StateDiffError::FailedToDeserializeStateDiff("Not enough bytes".to_string()),
        )?);
        self.offset += 32;

        Ok(res)
    }

    fn get_h256(&mut self) -> Result<H256, StateDiffError> {
        let res = H256::from_slice(self.bytes.get(self.offset..self.offset + 32).ok_or(
            StateDiffError::FailedToDeserializeStateDiff("Not enough bytes".to_string()),
        )?);
        self.offset += 32;

        Ok(res)
    }

    fn get_u8(&mut self) -> Result<u8, StateDiffError> {
        let res =
            self.bytes
                .get(self.offset)
                .ok_or(StateDiffError::FailedToDeserializeStateDiff(
                    "Not enough bytes".to_string(),
                ))?;
        self.offset += 1;

        Ok(*res)
    }

    fn get_u16(&mut self) -> Result<u16, StateDiffError> {
        let res = u16::from_be_bytes(
            self.bytes
                .get(self.offset..self.offset + 2)
                .ok_or(StateDiffError::FailedToDeserializeStateDiff(
                    "Not enough bytes".to_string(),
                ))?
                .try_into()
                .map_err(|_| {
                    StateDiffError::FailedToDeserializeStateDiff("Cannot parse u16".to_string())
                })?,
        );
        self.offset += 2;

        Ok(res)
    }

    fn get_u64(&mut self) -> Result<u64, StateDiffError> {
        let res = u64::from_be_bytes(
            self.bytes
                .get(self.offset..self.offset + 8)
                .ok_or(StateDiffError::FailedToDeserializeStateDiff(
                    "Not enough bytes".to_string(),
                ))?
                .try_into()
                .map_err(|_| {
                    StateDiffError::FailedToDeserializeStateDiff("Cannot parse u64".to_string())
                })?,
        );
        self.offset += 8;

        Ok(res)
    }

    fn get_bytes(&mut self, size: usize) -> Result<Bytes, StateDiffError> {
        let res = self.bytes.get(self.offset..self.offset + size).ok_or(
            StateDiffError::FailedToDeserializeStateDiff("Not enough bytes".to_string()),
        )?;
        self.offset += size;

        Ok(Bytes::copy_from_slice(res))
    }
}

/// Calculates nonce_diff between current and previous block.
pub fn get_nonce_diff(
    account_update: &AccountUpdate,
    db: &impl VmDatabase,
) -> Result<u16, StateDiffError> {
    // Get previous account_info either from store or cache
    let account_info = db.get_account_info(account_update.address)?;

    // Get previous nonce
    let prev_nonce = match account_info {
        Some(info) => info.nonce,
        None => 0,
    };

    // Get current nonce
    let new_nonce = if let Some(info) = account_update.info.clone() {
        info.nonce
    } else {
        prev_nonce
    };

    // Calculate nonce diff
    let nonce_diff = new_nonce
        .checked_sub(prev_nonce)
        .ok_or(StateDiffError::FailedToCalculateNonce)?
        .try_into()
        .map_err(StateDiffError::from)?;

    Ok(nonce_diff)
}

/// Prepare the state diff for the block.
pub fn prepare_state_diff(
    last_header: BlockHeader,
    db: &impl VmDatabase,
    l1messages: &[L1Message],
    deposits: &[PrivilegedL2Transaction],
    account_updates: Vec<AccountUpdate>,
) -> Result<StateDiff, StateDiffError> {
    let mut modified_accounts = BTreeMap::new();
    for account_update in account_updates {
        let nonce_diff = get_nonce_diff(&account_update, db)?;

        modified_accounts.insert(
            account_update.address,
            AccountStateDiff {
                new_balance: account_update.info.clone().map(|info| info.balance),
                nonce_diff,
                storage: account_update.added_storage.clone().into_iter().collect(),
                bytecode: account_update.code.clone(),
                bytecode_hash: None,
            },
        );
    }

    let state_diff = StateDiff {
        modified_accounts,
        version: StateDiff::default().version,
        last_header,
        l1_messages: l1messages.to_vec(),
        deposit_logs: deposits
            .iter()
            .map(|tx| DepositLog {
                address: match tx.to {
                    TxKind::Call(address) => address,
                    TxKind::Create => Address::zero(),
                },
                amount: tx.value,
                nonce: tx.nonce,
            })
            .collect(),
    };

    Ok(state_diff)
}
