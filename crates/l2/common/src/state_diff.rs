use std::collections::{BTreeMap, HashMap};

use bytes::Bytes;
use ethereum_types::Address;
use ethrex_common::types::{
    AccountInfo, AccountState, AccountUpdate, BlockHeader, Code, PrivilegedL2Transaction, TxKind,
    account_diff::{AccountDiffError, AccountStateDiff, Decoder, DecoderError},
    code_hash,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{error::StoreError, hash_address};
use ethrex_trie::{Trie, TrieError};
use ethrex_vm::{EvmError, VmDatabase};
use serde::{Deserialize, Serialize};

use crate::{messages::L1Message, privileged_transactions::PrivilegedTransactionLog};

/// The serialized length of a default l1message log
pub const L1MESSAGE_LOG_LEN: u64 = 84;

/// The serialized length of a default privileged transaction log
pub const PRIVILEGED_TX_LOG_LEN: u64 = 52;

/// The serialized lenght of a default block header
pub const BLOCK_HEADER_LEN: u64 = 136;

// State diff size for a simple transfer.
// Two `AccountUpdates` with new_balance, one of which also has nonce_diff.
pub const SIMPLE_TX_STATE_DIFF_SIZE: u64 = 108;

#[derive(Debug, thiserror::Error)]
pub enum StateDiffError {
    #[error("StateDiff failed to deserialize: {0}")]
    FailedToDeserializeStateDiff(String),
    #[error("StateDiff failed to serialize: {0}")]
    FailedToSerializeStateDiff(String),
    #[error("StateDiff unsupported version: {0}")]
    UnsupportedVersion(u8),
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
    #[error("Decoder Error: {0}")]
    DecoderError(#[from] DecoderError),
    #[error("AccountDiff Error: {0}")]
    AccountDiffError(#[from] AccountDiffError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDiff {
    pub version: u8,
    pub last_header: BlockHeader,
    pub modified_accounts: BTreeMap<Address, AccountStateDiff>,
    pub l1_messages: Vec<L1Message>,
    pub privileged_transactions: Vec<PrivilegedTransactionLog>,
}

impl Default for StateDiff {
    fn default() -> Self {
        StateDiff {
            version: 1,
            last_header: BlockHeader::default(),
            modified_accounts: BTreeMap::new(),
            l1_messages: Vec::new(),
            privileged_transactions: Vec::new(),
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

        let privileged_tx_len: u16 = self.privileged_transactions.len().try_into()?;
        encoded.extend(privileged_tx_len.to_be_bytes());
        for privileged_tx in self.privileged_transactions.iter() {
            let privileged_tx_encoded = privileged_tx.encode();
            encoded.extend(privileged_tx_encoded);
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
            let from = decoder.get_address()?;
            let data = decoder.get_h256()?;
            let index = decoder.get_u256()?;

            l1messages.push(L1Message {
                from,
                data_hash: data,
                message_id: index,
            });
        }

        let privileged_transactions_len = decoder.get_u16()?;

        let mut privileged_transactions = Vec::with_capacity(privileged_transactions_len.into());
        for _ in 0..privileged_transactions_len {
            let address = decoder.get_address()?;
            let amount = decoder.get_u256()?;

            privileged_transactions.push(PrivilegedTransactionLog {
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
            privileged_transactions,
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
                    code: diff.bytecode.clone().map(Code::from_bytecode),
                    added_storage: diff.storage.clone().into_iter().collect(),
                    removed_storage: false,
                },
            );
        }

        Ok(account_updates)
    }
}

/// Calculates nonce_diff between current and previous block.
pub fn get_nonce_diff(
    account_update: &AccountUpdate,
    db: &impl VmDatabase,
) -> Result<u16, StateDiffError> {
    // Get previous account_state either from store or cache
    let account_state = db.get_account_state(account_update.address)?;

    // Get previous nonce
    let prev_nonce = match account_state {
        Some(state) => state.nonce,
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
    privileged_transactions: &[PrivilegedL2Transaction],
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
                bytecode: account_update.code.map(|b| b.bytecode).clone(),
                bytecode_hash: None,
            },
        );
    }

    let state_diff = StateDiff {
        modified_accounts,
        version: StateDiff::default().version,
        last_header,
        l1_messages: l1messages.to_vec(),
        privileged_transactions: privileged_transactions
            .iter()
            .map(|tx| PrivilegedTransactionLog {
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

#[cfg(test)]
#[allow(clippy::as_conversions)]
mod tests {
    use ethrex_common::U256;

    use super::*;
    #[test]
    fn test_l1_message_size() {
        let l1_message_size = L1Message::default().encode().len() as u64;
        assert_eq!(L1MESSAGE_LOG_LEN, l1_message_size);
    }

    #[test]
    fn test_privileged_tx_log_size() {
        let privileged_tx_size = PrivilegedTransactionLog::default().encode().len() as u64;
        assert_eq!(PRIVILEGED_TX_LOG_LEN, privileged_tx_size);
    }

    #[test]
    fn test_block_header_size() {
        let block_header_size = encode_block_header(&BlockHeader::default()).len() as u64;
        assert_eq!(BLOCK_HEADER_LEN, block_header_size);
    }

    #[test]
    fn test_accounts_diff_size() {
        let empty_storage = BTreeMap::new();

        let account_diff_1 = AccountStateDiff {
            new_balance: Some(U256::from(1000)),
            nonce_diff: 1,
            storage: empty_storage.clone(),
            bytecode: None,
            bytecode_hash: None,
        };

        let account_diff_2 = AccountStateDiff {
            new_balance: Some(U256::from(1000)),
            nonce_diff: 0,
            storage: empty_storage,
            bytecode: None,
            bytecode_hash: None,
        };

        let account_diff_1_size = account_diff_1.encode(&Address::zero()).unwrap().len() as u64;
        let account_diff_2_size = account_diff_2.encode(&Address::zero()).unwrap().len() as u64;
        assert_eq!(
            SIMPLE_TX_STATE_DIFF_SIZE,
            account_diff_1_size + account_diff_2_size
        );
    }
}
