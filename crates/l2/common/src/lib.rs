use std::collections::{HashMap, BTreeMap};

use bytes::Bytes;
use ethereum_types::{Address, H256, U256};
use ethrex_common::types::{
    code_hash, AccountInfo, AccountState, AccountUpdate, BlockHeader, BlockNumber,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{error::StoreError, hash_address, Store};
use ethrex_trie::Trie;
use serde::{Deserialize, Serialize};

use super::errors::StateDiffError;

use lazy_static::lazy_static;

lazy_static! {
    /// The serialized length of a default withdrawal log
    pub static ref WITHDRAWAL_LOG_LEN: usize = WithdrawalLog::default().encode().len();

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
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AccountStateDiff {
    pub new_balance: Option<U256>,
    pub nonce_diff: u16,
    pub storage: BTreeMap<H256, U256>,
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

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct WithdrawalLog {
    pub address: Address,
    pub amount: U256,
    pub tx_hash: H256,
}

impl WithdrawalLog {
    pub fn encode(&self) -> Vec<u8> {
        let mut encoded = Vec::new();
        encoded.extend(self.address.0);
        encoded.extend_from_slice(&self.amount.to_big_endian());
        encoded.extend(&self.tx_hash.0);
        encoded
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct DepositLog {
    pub address: Address,
    pub amount: U256,
    pub nonce: u64,
}

impl DepositLog {
    pub fn encode(&self) -> Vec<u8> {
        let mut encoded = Vec::new();
        encoded.extend(self.address.0);
        encoded.extend_from_slice(&self.amount.to_big_endian());
        encoded
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct StateDiff {
    pub version: u8,
    pub last_header: BlockHeader,
    pub modified_accounts: BTreeMap<Address, AccountStateDiff>,
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
            last_header: BlockHeader::default(),
            modified_accounts: BTreeMap::new(),
            withdrawal_logs: Vec::new(),
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

        let withdrawal_len: u16 = self.withdrawal_logs.len().try_into()?;
        encoded.extend(withdrawal_len.to_be_bytes());
        for withdrawal in self.withdrawal_logs.iter() {
            let withdrawal_encoded = withdrawal.encode();
            encoded.extend(withdrawal_encoded);
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
            version,
            last_header,
            modified_accounts,
            withdrawal_logs,
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
/// Uses cache if provided to optimize account_info lookups.
pub async fn get_nonce_diff(
    account_update: &AccountUpdate,
    store: &Store,
    accounts_info_cache: Option<&mut HashMap<Address, Option<AccountInfo>>>,
    current_block_number: BlockNumber,
) -> Result<u16, StateDiffError> {
    // Get previous account_info either from store or cache
    let account_info = match accounts_info_cache {
        None => store
            .get_account_info(current_block_number - 1, account_update.address)
            .await
            .map_err(StoreError::from)?,
        Some(cache) => {
            account_info_from_cache(
                cache,
                store,
                account_update.address,
                current_block_number - 1,
            )
            .await?
        }
    };

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

/// Retrieves account info from cache or falls back to store.
/// Updates cache with fresh data if cache miss occurs.
async fn account_info_from_cache(
    cache: &mut HashMap<Address, Option<AccountInfo>>,
    store: &Store,
    address: Address,
    block_number: BlockNumber,
) -> Result<Option<AccountInfo>, StateDiffError> {
    let account_info = match cache.get(&address) {
        Some(account_info) => account_info.clone(),
        None => {
            let account_info = store
                .get_account_info(block_number, address)
                .await
                .map_err(StoreError::from)?;
            cache.insert(address, account_info.clone());
            account_info
        }
    };
    Ok(account_info)
}

/// Prepare the state diff for the block.
pub async fn prepare_state_diff(
    first_block_number: BlockNumber,
    last_header: BlockHeader,
    store: Store,
    withdrawals: &[(H256, Transaction)],
    deposits: &[PrivilegedL2Transaction],
    account_updates: Vec<AccountUpdate>,
) -> Result<StateDiff, StateDiffError> {
    let mut modified_accounts = BTreeMap::new();
    for account_update in account_updates {
        // If we want the state_diff of a batch, we will have to change the -1 with the `batch_size`
        // and we may have to keep track of the latestCommittedBlock (last block of the batch),
        // the batch_size and the latestCommittedBatch in the contract.
        let nonce_diff = get_nonce_diff(&account_update, &store, None, first_block_number).await?;

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
        withdrawal_logs: withdrawals
            .iter()
            .map(|(hash, tx)| WithdrawalLog {
                address: match tx.to() {
                    TxKind::Call(address) => address,
                    TxKind::Create => Address::zero(),
                },
                amount: tx.value(),
                tx_hash: *hash,
            })
            .collect(),
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

pub fn get_block_withdrawals(
    txs_and_receipts: &[(Transaction, Receipt)],
) -> Result<Vec<(H256, Transaction)>, StateDiffError> {
    let mut ret = vec![];

    for (tx, receipt) in txs_and_receipts {
        if is_withdrawal_l2(tx, receipt)? {
            ret.push((tx.compute_hash(), tx.clone()))
        }
    }
    Ok(ret)
}

pub fn get_block_deposits(
    txs_and_receipts: &[(Transaction, Receipt)],
) -> Vec<PrivilegedL2Transaction> {
    let deposits = txs_and_receipts
        .iter()
        .filter_map(|(tx, _)| match tx {
            Transaction::PrivilegedL2Transaction(tx) => Some(tx.clone()),
            _ => None,
        })
        .collect();

    deposits
}

pub fn is_withdrawal_l2(tx: &Transaction, receipt: &Receipt) -> Result<bool, StateDiffError> {
    pub const COMMON_BRIDGE_L2_ADDRESS: Address = H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0xff, 0xff,
    ]);

    let withdrawal_event_selector = keccak("WithdrawalInitiated(address,address,uint256)");

    let is_withdrawal = match tx.to() {
        TxKind::Call(to) if to == COMMON_BRIDGE_L2_ADDRESS => receipt.logs.iter().any(|log| {
            log.topics
                .iter()
                .any(|topic| *topic == withdrawal_event_selector)
        }),
        _ => false,
    };
    Ok(is_withdrawal)
}

pub fn is_deposit_l2(tx: &Transaction) -> bool {
    matches!(tx, Transaction::PrivilegedL2Transaction(_tx))
}

pub async fn get_tx_and_receipts(
    block: &Block,
    store: Store,
) -> Result<Vec<(Transaction, Receipt)>, StateDiffError> {
    // Get block transactions and receipts
    let mut txs_and_receipts = vec![];
    for (index, tx) in block.body.transactions.iter().enumerate() {
        let receipt = store
            .get_receipt(block.header.number, index.try_into()?)
            .await?
            .ok_or(StateDiffError::InternalError(
                "Transactions in a block should have a receipt".to_owned(),
            ))?;
        txs_and_receipts.push((tx.clone(), receipt));
    }
    Ok(txs_and_receipts)
}
