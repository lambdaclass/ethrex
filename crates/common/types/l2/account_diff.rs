// This file needs to be accessible from both the `vm` and `L2` crates.

use bytes::Bytes;
use ethereum_types::{Address, H256, U256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::debug;

#[derive(Debug, thiserror::Error)]
pub enum DecoderError {
    #[error("Decoder failed to deserialize: {0}")]
    FailedToDeserialize(String),
    #[error("StateDiff failed to deserialize: {0}")]
    FailedToDeserializeStateDiff(String),
}

#[derive(Debug, thiserror::Error)]
pub enum AccountDiffError {
    #[error("StateDiff invalid account state diff type: {0}")]
    InvalidAccountStateDiffType(u8),
    #[error("Both bytecode and bytecode hash are set")]
    BytecodeAndBytecodeHashSet,
    #[error("The length of the vector is too big to fit in u16: {0}")]
    LengthTooBig(#[from] core::num::TryFromIntError),
    #[error("Empty account diff")]
    EmptyAccountDiff,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
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

impl TryFrom<u8> for AccountStateDiffType {
    type Error = AccountDiffError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(AccountStateDiffType::NewBalance),
            2 => Ok(AccountStateDiffType::NonceDiff),
            4 => Ok(AccountStateDiffType::Storage),
            8 => Ok(AccountStateDiffType::Bytecode),
            16 => Ok(AccountStateDiffType::BytecodeHash),
            _ => Err(AccountDiffError::InvalidAccountStateDiffType(value)),
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

pub fn get_accounts_diff_size(
    account_diffs: &BTreeMap<Address, AccountStateDiff>,
) -> Result<u64, AccountDiffError> {
    let mut new_accounts_diff_size = 0;

    for (address, diff) in account_diffs.iter() {
        let encoded = match diff.encode(address) {
            Ok(encoded) => encoded,
            Err(AccountDiffError::EmptyAccountDiff) => {
                debug!("Skipping empty account diff for address: {address}");
                continue;
            }
            Err(e) => {
                return Err(e);
            }
        };
        let encoded_len: u64 = encoded.len().try_into()?;
        new_accounts_diff_size += encoded_len;
    }
    Ok(new_accounts_diff_size)
}

impl AccountStateDiff {
    pub fn encode(&self, address: &Address) -> Result<Vec<u8>, AccountDiffError> {
        if self.bytecode.is_some() && self.bytecode_hash.is_some() {
            return Err(AccountDiffError::BytecodeAndBytecodeHashSet);
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
            let storage_len: u16 = self.storage.len().try_into()?;
            r#type += r_type;
            encoded.extend(storage_len.to_be_bytes());
            for (key, value) in &self.storage {
                encoded.extend_from_slice(&key.0);
                encoded.extend_from_slice(&value.to_big_endian());
            }
        }

        if let Some(bytecode) = &self.bytecode {
            let r_type: u8 = AccountStateDiffType::Bytecode.into();
            let bytecode_len: u16 = bytecode.len().try_into()?;
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
            return Err(AccountDiffError::EmptyAccountDiff);
        }

        let mut result = Vec::with_capacity(1 + address.0.len() + encoded.len());
        result.extend(r#type.to_be_bytes());
        result.extend(address.0);
        result.extend(encoded);

        Ok(result)
    }

    /// Returns a tuple of the number of bytes read, the address of the account
    /// and the decoded `AccountStateDiff`
    pub fn decode(bytes: &[u8]) -> Result<(usize, Address, Self), DecoderError> {
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

pub struct Decoder {
    bytes: Bytes,
    offset: usize,
}

impl Decoder {
    pub fn new(bytes: &[u8]) -> Self {
        Decoder {
            bytes: Bytes::copy_from_slice(bytes),
            offset: 0,
        }
    }

    pub fn consumed(&self) -> usize {
        self.offset
    }

    pub fn advance(&mut self, size: usize) {
        self.offset += size;
    }

    pub fn get_address(&mut self) -> Result<Address, DecoderError> {
        let res = Address::from_slice(self.bytes.get(self.offset..self.offset + 20).ok_or(
            DecoderError::FailedToDeserializeStateDiff("Not enough bytes".to_string()),
        )?);
        self.offset += 20;

        Ok(res)
    }

    pub fn get_u256(&mut self) -> Result<U256, DecoderError> {
        let res = U256::from_big_endian(self.bytes.get(self.offset..self.offset + 32).ok_or(
            DecoderError::FailedToDeserializeStateDiff("Not enough bytes".to_string()),
        )?);
        self.offset += 32;

        Ok(res)
    }

    pub fn get_h256(&mut self) -> Result<H256, DecoderError> {
        let res = H256::from_slice(self.bytes.get(self.offset..self.offset + 32).ok_or(
            DecoderError::FailedToDeserializeStateDiff("Not enough bytes".to_string()),
        )?);
        self.offset += 32;

        Ok(res)
    }

    pub fn get_u8(&mut self) -> Result<u8, DecoderError> {
        let res = self
            .bytes
            .get(self.offset)
            .ok_or(DecoderError::FailedToDeserializeStateDiff(
                "Not enough bytes".to_string(),
            ))?;
        self.offset += 1;

        Ok(*res)
    }

    pub fn get_u16(&mut self) -> Result<u16, DecoderError> {
        let res = u16::from_be_bytes(
            self.bytes
                .get(self.offset..self.offset + 2)
                .ok_or(DecoderError::FailedToDeserializeStateDiff(
                    "Not enough bytes".to_string(),
                ))?
                .try_into()
                .map_err(|_| {
                    DecoderError::FailedToDeserializeStateDiff("Cannot parse u16".to_string())
                })?,
        );
        self.offset += 2;

        Ok(res)
    }

    pub fn get_u64(&mut self) -> Result<u64, DecoderError> {
        let res = u64::from_be_bytes(
            self.bytes
                .get(self.offset..self.offset + 8)
                .ok_or(DecoderError::FailedToDeserializeStateDiff(
                    "Not enough bytes".to_string(),
                ))?
                .try_into()
                .map_err(|_| {
                    DecoderError::FailedToDeserializeStateDiff("Cannot parse u64".to_string())
                })?,
        );
        self.offset += 8;

        Ok(res)
    }

    pub fn get_bytes(&mut self, size: usize) -> Result<Bytes, DecoderError> {
        let res = self.bytes.get(self.offset..self.offset + size).ok_or(
            DecoderError::FailedToDeserializeStateDiff("Not enough bytes".to_string()),
        )?;
        self.offset += size;

        Ok(Bytes::copy_from_slice(res))
    }
}
