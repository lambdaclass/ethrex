use std::collections::HashMap;

use bytes::Bytes;
use ethereum_types::{Address, H256, U256};
use ethrex_common::types::{AccountInfo, AccountState, BlockHeader};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{hash_address, AccountUpdate};
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

pub trait AccountStateDiffCmp {
    fn is(&self, r#type: AccountStateDiffType) -> bool;
}

impl AccountStateDiffCmp for u8 {
    fn is(&self, r#type: AccountStateDiffType) -> bool {
        return self & r#type as u8 != 0;
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

        for withdrawal in self.withdrawal_logs.iter() {
            encoded.extend(withdrawal.address.0);
            encoded.extend_from_slice(&withdrawal.amount.to_big_endian());
            encoded.extend(&withdrawal.tx_hash.0);
        }

        for deposit in self.deposit_logs.iter() {
            encoded.extend(deposit.address.0);
            encoded.extend_from_slice(&deposit.amount.to_big_endian());
        }

        Ok(Bytes::from(encoded))
    }

    pub fn decode(bytes: Bytes) -> Result<Self, StateDiffError> {
        let mut offset = 0;
        if bytes[offset] != 0x01 {
            return Err(StateDiffError::UnsupportedVersion(bytes[offset]));
        }
        offset += 1;

        // Header fields
        let transactions_root = H256::from_slice(&bytes[offset..offset + 32]);
        offset += 32;
        let receipts_root = H256::from_slice(&bytes[offset..offset + 32]);
        offset += 32;
        let gas_limit = u64::from_be_bytes(bytes[offset..offset + 8].try_into().map_err(|_| {
            StateDiffError::FailedToDeserializeStateDiff("Invalid gas limit".to_string())
        })?);
        offset += 8;
        let gas_used = u64::from_be_bytes(bytes[offset..offset + 8].try_into().map_err(|_| {
            StateDiffError::FailedToDeserializeStateDiff("Invalid gas used".to_string())
        })?);
        offset += 8;
        let timestamp = u64::from_be_bytes(bytes[offset..offset + 8].try_into().map_err(|_| {
            StateDiffError::FailedToDeserializeStateDiff("Invalid timestamp".to_string())
        })?);
        offset += 8;
        let base_fee_per_gas =
            u64::from_be_bytes(bytes[offset..offset + 8].try_into().map_err(|_| {
                StateDiffError::FailedToDeserializeStateDiff("Invalid base fee per gas".to_string())
            })?);
        offset += 8;

        let accounts_updated = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
        offset += 2;

        let mut modified_accounts = HashMap::with_capacity(accounts_updated as usize);
        for _ in 0..accounts_updated {
            let (bytes_read, address, account_diff) =
                AccountStateDiff::decode(bytes[offset..].to_vec().into())?;
            offset += bytes_read;
            modified_accounts.insert(address, account_diff);
        }

        let withdrawal_logs_len = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
        offset += 2;

        let mut withdrawal_logs = Vec::with_capacity(withdrawal_logs_len as usize);
        for _ in 0..withdrawal_logs_len {
            let address = Address::from_slice(&bytes[offset..offset + 20]);
            offset += 20;
            let amount = U256::from_big_endian(&bytes[offset..offset + 32]);
            offset += 32;
            let tx_hash = H256::from_slice(&bytes[offset..offset + 32]);
            offset += 32;

            withdrawal_logs.push(WithdrawalLog {
                address,
                amount,
                tx_hash,
            });
        }

        let deposit_logs_len = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
        offset += 2;

        let mut deposit_logs = Vec::with_capacity(deposit_logs_len as usize);
        for _ in 0..deposit_logs_len {
            let address = Address::from_slice(&bytes[offset..offset + 20]);
            offset += 20;
            let amount = U256::from_big_endian(&bytes[offset..offset + 32]);
            offset += 32;

            deposit_logs.push(DepositLog {
                address,
                amount,
                nonce: Default::default(),
            });
        }

        Ok(Self {
            version: 1,
            header: BlockHeader {
                transactions_root,
                receipts_root,
                gas_limit,
                gas_used,
                timestamp,
                base_fee_per_gas: Some(base_fee_per_gas),
                ..Default::default()
            },
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
                .map_err(|e| StateDiffError::DbError(e))?
            {
                Some(rlp) => AccountState::decode(&rlp)
                    .map_err(|e| StateDiffError::FailedToDeserializeStateDiff(e.to_string()))?,
                None => AccountState::default(),
            };

            let balance = diff.new_balance.unwrap_or(account_state.balance);
            let nonce = account_state.nonce + diff.nonce_diff as u64;
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
            let bytecode_len: u16 = self
                .storage
                .len()
                .try_into()
                .map_err(StateDiffError::from)?;
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
    pub fn decode(bytes: Bytes) -> Result<(usize, Address, Self), StateDiffError> {
        let mut offset = 0;

        let update_type = bytes[offset];
        offset += 1;

        let address = Address::from_slice(&bytes[offset..offset + 20]);
        offset += 20;

        let new_balance = if update_type.is(AccountStateDiffType::NewBalance) {
            let balance = U256::from_big_endian(&bytes[offset..offset + 32]);
            offset += 32;
            Some(balance)
        } else {
            None
        };

        let nonce_diff = if update_type.is(AccountStateDiffType::NonceDiff) {
            let nonce = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
            offset += 2;
            Some(nonce)
        } else {
            None
        };

        let mut storage_diff = HashMap::new();
        if update_type.is(AccountStateDiffType::Storage) {
            let storage_slots_updated = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
            offset += 2;
            storage_diff.reserve(storage_slots_updated as usize);

            for _ in 0..storage_slots_updated {
                let key = H256::from_slice(&bytes[offset..offset + 32]);
                offset += 32;
                let new_value = U256::from_big_endian(&bytes[offset..offset + 32]);
                offset += 32;

                storage_diff.insert(key, new_value);
            }
        }

        let bytecode = if update_type.is(AccountStateDiffType::Bytecode) {
            let bytecode_len = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
            offset += 2;

            let bytecode = bytes[offset..offset + bytecode_len as usize]
                .to_vec()
                .into();
            offset += bytecode_len as usize;

            Some(bytecode)
        } else {
            None
        };

        let bytecode_hash = if update_type.is(AccountStateDiffType::BytecodeHash) {
            let bytecode_hash = H256::from_slice(&bytes[offset..offset + 32]);
            offset += 32;
            Some(bytecode_hash)
        } else {
            None
        };

        Ok((
            offset,
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
