use std::collections::HashMap;

use bytes::Bytes;
use ethereum_types::{Address, H256, U256};
use ethrex_common::types::{AccountInfo, AccountState, BlockHeader, BlockNumber};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{error::StoreError, hash_address, AccountUpdate, Store};
use ethrex_trie::Trie;

use super::errors::StateDiffError;

#[derive(Clone)]
pub struct AccountStateDiff {
    pub new_balance: Option<U256>,
    pub nonce_diff: u16,
    pub storage: HashMap<H256, U256>,
    pub bytecode: Option<Bytes>,
    pub bytecode_hash: Option<H256>,
}

#[derive(Clone, Copy)]
pub enum AccountStateDiffType {
    NewBalance = 1,
    NonceDiff = 2,
    Storage = 4,
    Bytecode = 8,
    BytecodeHash = 16,
}

#[derive(Clone)]
pub struct WithdrawalLog {
    pub address: Address,
    pub amount: U256,
    pub tx_hash: H256,
}

#[derive(Clone)]
pub struct DepositLog {
    pub address: Address,
    pub amount: U256,
    pub nonce: u64,
}

#[derive(Clone)]
pub struct StateDiff {
    pub version: u8,
    pub header: BlockHeader,
    pub modified_accounts: HashMap<Address, AccountStateDiff>,
    pub withdrawal_logs: Vec<WithdrawalLog>,
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
            header: BlockHeader::default(),
            modified_accounts: HashMap::new(),
            withdrawal_logs: Vec::new(),
            deposit_logs: Vec::new(),
        }
    }
}

impl StateDiff {
    pub fn encode(&self) -> Result<Bytes, StateDiffError> {
        if self.version != 1 {
            return Err(StateDiffError::UnsupportedVersion(self.version));
        }

        let mut encoded: Vec<u8> = Vec::new();
        encoded.push(self.version);

        // Header fields
        encoded.extend(self.header.transactions_root.0);
        encoded.extend(self.header.receipts_root.0);
        encoded.extend(self.header.gas_limit.to_be_bytes());
        encoded.extend(self.header.gas_used.to_be_bytes());
        encoded.extend(self.header.timestamp.to_be_bytes());
        encoded.extend(self.header.base_fee_per_gas.unwrap_or(0).to_be_bytes());

        let modified_accounts_len: u16 = self
            .modified_accounts
            .len()
            .try_into()
            .map_err(StateDiffError::from)?;
        encoded.extend(modified_accounts_len.to_be_bytes());

        for (address, diff) in &self.modified_accounts {
            let (r#type, diff_encoded) = diff.encode()?;
            encoded.extend(r#type.to_be_bytes());
            encoded.extend(address.0);
            encoded.extend(diff_encoded);
        }

        let withdrawal_len: u16 = self.withdrawal_logs.len().try_into()?;
        encoded.extend(withdrawal_len.to_be_bytes());
        for withdrawal in self.withdrawal_logs.iter() {
            encoded.extend(withdrawal.address.0);
            encoded.extend_from_slice(&withdrawal.amount.to_big_endian());
            encoded.extend(&withdrawal.tx_hash.0);
        }

        let deposits_len: u16 = self.deposit_logs.len().try_into()?;
        encoded.extend(deposits_len.to_be_bytes());
        for deposit in self.deposit_logs.iter() {
            encoded.extend(deposit.address.0);
            encoded.extend_from_slice(&deposit.amount.to_big_endian());
        }

        Ok(Bytes::from(encoded))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, StateDiffError> {
        let mut decoder = Decoder::new(bytes);

        let version = decoder.get_u8()?;
        if version != 0x01 {
            return Err(StateDiffError::UnsupportedVersion(version));
        }

        // Header fields
        let transactions_root = decoder.get_h256()?;
        let receipts_root = decoder.get_h256()?;
        let gas_limit = decoder.get_u64()?;
        let gas_used = decoder.get_u64()?;
        let timestamp = decoder.get_u64()?;
        let base_fee_per_gas = decoder.get_u64()?;

        // Accounts diff
        let modified_accounts_len = decoder.get_u16()?;

        let mut modified_accounts = HashMap::with_capacity(modified_accounts_len.into());
        for _ in 0..modified_accounts_len {
            let next_bytes = bytes.get(decoder.consumed()..).ok_or(
                StateDiffError::FailedToSerializeStateDiff("Not enough bytes".to_string()),
            )?;
            let (bytes_read, address, account_diff) = AccountStateDiff::decode(next_bytes)?;
            decoder.advance(bytes_read);
            modified_accounts.insert(address, account_diff);
        }

        let withdrawal_logs_len = decoder.get_u16()?;

        let mut withdrawal_logs = Vec::with_capacity(withdrawal_logs_len.into());
        for _ in 0..withdrawal_logs_len {
            let address = decoder.get_address()?;
            let amount = decoder.get_u256()?;
            let tx_hash = decoder.get_h256()?;

            withdrawal_logs.push(WithdrawalLog {
                address,
                amount,
                tx_hash,
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
            header: BlockHeader {
                transactions_root,
                receipts_root,
                gas_limit,
                gas_used,
                timestamp,
                base_fee_per_gas: Some(base_fee_per_gas),
                ..Default::default()
            },
            version,
            modified_accounts,
            withdrawal_logs,
            deposit_logs,
        })
    }

    pub fn to_account_updates(
        &self,
        prev_state: &Trie,
    ) -> Result<Vec<AccountUpdate>, StateDiffError> {
        let mut account_updates = Vec::new();

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
            let bytecode_hash = diff.bytecode_hash.unwrap_or(account_state.code_hash);

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

            account_updates.push(AccountUpdate {
                address: *address,
                removed: false,
                info: account_info,
                code: diff.bytecode.clone(),
                added_storage: diff.storage.clone(),
            });
        }

        Ok(account_updates)
    }
}

impl AccountStateDiff {
    pub fn encode(&self) -> Result<(u8, Bytes), StateDiffError> {
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

        Ok((r#type, Bytes::from(encoded)))
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

        let mut storage_diff = HashMap::new();
        if AccountStateDiffType::Storage.is_in(update_type) {
            let storage_slots_updated = decoder.get_u16()?;
            storage_diff.reserve(storage_slots_updated.into());

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

pub fn get_nonce_diff(
    account_update: &AccountUpdate,
    store: &Store,
    current_block_number: BlockNumber,
) -> Result<u16, StateDiffError> {
    let prev_nonce = match store
        .get_account_info(current_block_number - 1, account_update.address)
        .map_err(StoreError::from)?
    {
        Some(acc) => acc.nonce,
        None => 0,
    };

    let new_nonce = if let Some(info) = account_update.info.clone() {
        info.nonce
    } else {
        prev_nonce
    };
    let nonce_diff = new_nonce
        .checked_sub(prev_nonce)
        .ok_or(StateDiffError::FailedToCalculateNonce)?
        .try_into()
        .map_err(StateDiffError::from)?;
    Ok(nonce_diff)
}
