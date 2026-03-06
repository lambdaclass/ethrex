use std::collections::BTreeMap;

use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_trie::Trie;
use librlp::{RlpDecode, RlpEncode, RlpError};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use super::GenesisAccount;
use crate::{
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    utils::keccak,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct Code {
    // hash is only used for bytecodes stored in the DB, either for reading it from the DB
    // or with the CODEHASH opcode, which needs an account address as argument and
    // thus only accessed persisted bytecodes.
    // We use a bogus H256::zero() value for initcodes as there is no way for the VM or
    // endpoints to access that hash, saving one expensive Keccak hash.
    pub hash: H256,
    pub bytecode: Bytes,
    // TODO: Consider using Arc<[u32]> (needs to enable serde rc feature)
    // The valid addresses are 32-bit because, despite EIP-3860 restricting initcode size,
    // this does not apply to previous forks. This is tested in the EEST tests, which would
    // panic in debug mode.
    pub jump_targets: Vec<u32>,
}

impl Code {
    // SAFETY: hash will be stored as-is, so it either needs to match
    // the real code hash (i.e. it was precomputed and we're reusing)
    // or never be read (e.g. for initcode).
    pub fn from_bytecode_unchecked(code: Bytes, hash: H256) -> Self {
        let jump_targets = Self::compute_jump_targets(&code);
        Self {
            hash,
            bytecode: code,
            jump_targets,
        }
    }

    pub fn from_bytecode(code: Bytes) -> Self {
        let jump_targets = Self::compute_jump_targets(&code);
        Self {
            hash: keccak(code.as_ref()),
            bytecode: code,
            jump_targets,
        }
    }

    fn compute_jump_targets(code: &[u8]) -> Vec<u32> {
        debug_assert!(code.len() <= u32::MAX as usize);
        let mut targets = Vec::new();
        let mut i = 0;
        while i < code.len() {
            // TODO: we don't use the constants from the vm module to avoid a circular dependency
            match code[i] {
                // OP_JUMPDEST
                0x5B => {
                    targets.push(i as u32);
                }
                // OP_PUSH1..32
                c @ 0x60..0x80 => {
                    // OP_PUSH0
                    i += (c - 0x5F) as usize;
                }
                _ => (),
            }
            i += 1;
        }
        targets
    }

    /// Estimates the size of the Code struct in bytes
    /// (including stack size and heap allocation).
    ///
    /// Note: This is an estimation and may not be exact.
    ///
    /// # Returns
    ///
    /// usize - Estimated size in bytes
    pub fn size(&self) -> usize {
        let hash_size = size_of::<H256>();
        let bytes_size = size_of::<Bytes>();
        let vec_size = size_of::<Vec<u32>>() + self.jump_targets.len() * size_of::<u32>();
        hash_size + bytes_size + vec_size
    }
}

impl AsRef<Bytes> for Code {
    fn as_ref(&self) -> &Bytes {
        &self.bytecode
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeMetadata {
    pub length: u64,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub info: AccountInfo,
    pub code: Code,
    pub storage: FxHashMap<H256, U256>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Eq, Hash)]
pub struct AccountInfo {
    pub code_hash: H256,
    pub balance: U256,
    pub nonce: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountState {
    pub nonce: u64,
    pub balance: U256,
    pub storage_root: H256,
    pub code_hash: H256,
}

/// A slim codec for an [`AccountState`].
///
/// The slim codec will optimize both the [storage root](AccountState::storage_root) and the
/// [code hash](AccountState::code_hash)'s encoding so that it does not take space when empty.
///
/// The correct way to use it is to wrap the [`AccountState`] and encode it using this codec, and
/// not to store the codec as a field in a struct.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AccountStateSlimCodec(pub AccountState);

impl Default for AccountInfo {
    fn default() -> Self {
        Self {
            code_hash: *EMPTY_KECCACK_HASH,
            balance: Default::default(),
            nonce: Default::default(),
        }
    }
}

impl Default for AccountState {
    fn default() -> Self {
        Self {
            nonce: Default::default(),
            balance: Default::default(),
            storage_root: *EMPTY_TRIE_HASH,
            code_hash: *EMPTY_KECCACK_HASH,
        }
    }
}

impl Default for Code {
    fn default() -> Self {
        Self {
            bytecode: Bytes::new(),
            hash: *EMPTY_KECCACK_HASH,
            jump_targets: Vec::new(),
        }
    }
}

impl From<GenesisAccount> for Account {
    fn from(genesis: GenesisAccount) -> Self {
        let code = Code::from_bytecode(genesis.code);
        Self {
            info: AccountInfo {
                code_hash: code.hash,
                balance: genesis.balance,
                nonce: genesis.nonce,
            },
            code,
            storage: genesis
                .storage
                .iter()
                .map(|(k, v)| (H256(k.to_big_endian()), *v))
                .collect(),
        }
    }
}

pub fn code_hash(code: &Bytes) -> H256 {
    keccak(code.as_ref())
}

impl RlpEncode for AccountInfo {
    fn encode(&self, buf: &mut librlp::RlpBuf) {
        buf.list(|buf| {
            self.code_hash.encode(buf);
            self.balance.encode(buf);
            self.nonce.encode(buf);
        });
    }

    fn encoded_length(&self) -> usize {
        crate::constants::list_encoded_length(
            self.code_hash.encoded_length()
                + self.balance.encoded_length()
                + self.nonce.encoded_length(),
        )
    }
}

impl RlpDecode for AccountInfo {
    fn decode(buf: &mut &[u8]) -> Result<Self, RlpError> {
        let header = librlp::Header::decode(buf)?;
        if !header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut payload = &buf[..header.payload_length];
        let code_hash = RlpDecode::decode(&mut payload)?;
        let balance = RlpDecode::decode(&mut payload)?;
        let nonce = RlpDecode::decode(&mut payload)?;
        *buf = &buf[header.payload_length..];
        Ok(AccountInfo {
            code_hash,
            balance,
            nonce,
        })
    }
}

impl RlpEncode for AccountState {
    fn encode(&self, buf: &mut librlp::RlpBuf) {
        buf.list(|buf| {
            self.nonce.encode(buf);
            self.balance.encode(buf);
            self.storage_root.encode(buf);
            self.code_hash.encode(buf);
        });
    }

    fn encoded_length(&self) -> usize {
        crate::constants::list_encoded_length(
            self.nonce.encoded_length()
                + self.balance.encoded_length()
                + self.storage_root.encoded_length()
                + self.code_hash.encoded_length(),
        )
    }
}

impl RlpDecode for AccountState {
    fn decode(buf: &mut &[u8]) -> Result<Self, RlpError> {
        let header = librlp::Header::decode(buf)?;
        if !header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut payload = &buf[..header.payload_length];
        let nonce = RlpDecode::decode(&mut payload)?;
        let balance = RlpDecode::decode(&mut payload)?;
        let storage_root = RlpDecode::decode(&mut payload)?;
        let code_hash = RlpDecode::decode(&mut payload)?;
        *buf = &buf[header.payload_length..];
        Ok(AccountState {
            nonce,
            balance,
            storage_root,
            code_hash,
        })
    }
}

impl RlpEncode for AccountStateSlimCodec {
    fn encode(&self, buf: &mut librlp::RlpBuf) {
        struct StorageRootCodec<'a>(&'a H256);
        impl RlpEncode for StorageRootCodec<'_> {
            fn encode(&self, buf: &mut librlp::RlpBuf) {
                let data = if *self.0 != *EMPTY_TRIE_HASH {
                    self.0.as_bytes()
                } else {
                    &[]
                };
                data.encode(buf);
            }

            fn encoded_length(&self) -> usize {
                let data = if *self.0 != *EMPTY_TRIE_HASH {
                    self.0.as_bytes()
                } else {
                    &[] as &[u8]
                };
                data.encoded_length()
            }
        }

        struct CodeHashCodec<'a>(&'a H256);
        impl RlpEncode for CodeHashCodec<'_> {
            fn encode(&self, buf: &mut librlp::RlpBuf) {
                let data = if *self.0 != *EMPTY_KECCACK_HASH {
                    self.0.as_bytes()
                } else {
                    &[]
                };
                data.encode(buf);
            }

            fn encoded_length(&self) -> usize {
                let data = if *self.0 != *EMPTY_KECCACK_HASH {
                    self.0.as_bytes()
                } else {
                    &[] as &[u8]
                };
                data.encoded_length()
            }
        }

        buf.list(|buf| {
            self.0.nonce.encode(buf);
            self.0.balance.encode(buf);
            StorageRootCodec(&self.0.storage_root).encode(buf);
            CodeHashCodec(&self.0.code_hash).encode(buf);
        });
    }

    fn encoded_length(&self) -> usize {
        let sr_len = if self.0.storage_root != *EMPTY_TRIE_HASH {
            self.0.storage_root.as_bytes().encoded_length()
        } else {
            (&[] as &[u8]).encoded_length()
        };
        let ch_len = if self.0.code_hash != *EMPTY_KECCACK_HASH {
            self.0.code_hash.as_bytes().encoded_length()
        } else {
            (&[] as &[u8]).encoded_length()
        };
        crate::constants::list_encoded_length(
            self.0.nonce.encoded_length() + self.0.balance.encoded_length() + sr_len + ch_len,
        )
    }
}

impl RlpDecode for AccountStateSlimCodec {
    fn decode(buf: &mut &[u8]) -> Result<Self, RlpError> {
        let header = librlp::Header::decode(buf)?;
        if !header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut payload = &buf[..header.payload_length];
        let nonce = RlpDecode::decode(&mut payload)?;
        let balance = RlpDecode::decode(&mut payload)?;

        // Custom decode for storage_root: 0x80 => empty trie hash, 0xA0 + 32 bytes => H256
        let storage_root = {
            let first = payload.first().ok_or(RlpError::InputTooShort)?;
            match *first {
                0x80 => {
                    payload = &payload[1..];
                    *EMPTY_TRIE_HASH
                }
                0xA0 => {
                    payload = &payload[1..];
                    let (data, rest) = payload
                        .split_first_chunk::<32>()
                        .ok_or(RlpError::InputTooShort)?;
                    payload = rest;
                    H256(*data)
                }
                _ => return Err(RlpError::InputTooShort),
            }
        };

        // Custom decode for code_hash: 0x80 => empty keccak hash, 0xA0 + 32 bytes => H256
        let code_hash = {
            let first = payload.first().ok_or(RlpError::InputTooShort)?;
            match *first {
                0x80 => {
                    *EMPTY_KECCACK_HASH
                }
                0xA0 => {
                    payload = &payload[1..];
                    let (data, _rest) = payload
                        .split_first_chunk::<32>()
                        .ok_or(RlpError::InputTooShort)?;
                    H256(*data)
                }
                _ => return Err(RlpError::InputTooShort),
            }
        };

        *buf = &buf[header.payload_length..];
        Ok(Self(AccountState {
            nonce,
            balance,
            storage_root,
            code_hash,
        }))
    }
}

pub fn compute_storage_root(storage: &BTreeMap<U256, U256>) -> H256 {
    let iter = storage.iter().filter_map(|(k, v)| {
        (!v.is_zero()).then_some((keccak_hash(k.to_big_endian()).to_vec(), v.to_rlp()))
    });
    Trie::compute_hash_from_unsorted_iter(iter)
}

impl From<&GenesisAccount> for AccountState {
    fn from(value: &GenesisAccount) -> Self {
        AccountState {
            nonce: value.nonce,
            balance: value.balance,
            storage_root: compute_storage_root(&value.storage),
            code_hash: code_hash(&value.code),
        }
    }
}

impl Account {
    pub fn new(balance: U256, code: Code, nonce: u64, storage: FxHashMap<H256, U256>) -> Self {
        Self {
            info: AccountInfo {
                balance,
                code_hash: code.hash,
                nonce,
            },
            code,
            storage,
        }
    }
}

impl AccountInfo {
    pub fn is_empty(&self) -> bool {
        self.balance.is_zero() && self.nonce == 0 && self.code_hash == *EMPTY_KECCACK_HASH
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_code_hash() {
        let empty_code = Bytes::new();
        let hash = code_hash(&empty_code);
        assert_eq!(
            hash,
            H256::from_str("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470")
                .unwrap()
        )
    }
}
