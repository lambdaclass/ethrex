use std::collections::BTreeMap;

use bytes::{BufMut, Bytes};
use ethereum_types::{H256, U256};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_trie::Trie;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

use super::GenesisAccount;
use crate::{
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    utils::keccak,
};

mod bytecode;
pub mod gas_costs;

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
    pub static_costs: Vec<(u32, u64)>,
}

impl Code {
    // SAFETY: hash will be stored as-is, so it either needs to match
    // the real code hash (i.e. it was precomputed and we're reusing)
    // or never be read (e.g. for initcode).
    pub fn from_bytecode_unchecked(code: Bytes, hash: H256) -> Self {
        let (jump_targets, static_costs) = self::bytecode::process_bytecode(&code);
        Self {
            hash,
            bytecode: code,
            jump_targets,
            static_costs,
        }
    }

    pub fn from_bytecode(code: Bytes) -> Self {
        let (jump_targets, static_costs) = self::bytecode::process_bytecode(&code);
        Self {
            hash: keccak(code.as_ref()),
            bytecode: code,
            jump_targets,
            static_costs,
        }
    }

    /// Look up the pre-computed static gas cost of the basic block
    /// starting at `pc`. Returns 0 when `pc` does not head a block
    /// (e.g. unreachable code after a terminator).
    pub fn block_cost(&self, pc: u32) -> u64 {
        self.static_costs
            .binary_search_by_key(&pc, |&(addr, _)| addr)
            .map_or(0, |idx| self.static_costs[idx].1)
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
            static_costs: Vec::new(),
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

impl RLPEncode for AccountInfo {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.code_hash)
            .encode_field(&self.balance)
            .encode_field(&self.nonce)
            .finish();
    }
}

impl RLPDecode for AccountInfo {
    fn decode_unfinished(rlp: &[u8]) -> Result<(AccountInfo, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (code_hash, decoder) = decoder.decode_field("code_hash")?;
        let (balance, decoder) = decoder.decode_field("balance")?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let account_info = AccountInfo {
            code_hash,
            balance,
            nonce,
        };
        Ok((account_info, decoder.finish()?))
    }
}

impl RLPEncode for AccountState {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.nonce)
            .encode_field(&self.balance)
            .encode_field(&self.storage_root)
            .encode_field(&self.code_hash)
            .finish();
    }
}

impl RLPDecode for AccountState {
    fn decode_unfinished(rlp: &[u8]) -> Result<(AccountState, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let (balance, decoder) = decoder.decode_field("balance")?;
        let (storage_root, decoder) = decoder.decode_field("storage_root")?;
        let (code_hash, decoder) = decoder.decode_field("code_hash")?;
        let state = AccountState {
            nonce,
            balance,
            storage_root,
            code_hash,
        };
        Ok((state, decoder.finish()?))
    }
}

impl RLPEncode for AccountStateSlimCodec {
    fn encode(&self, buf: &mut dyn BufMut) {
        struct StorageRootCodec<'a>(&'a H256);
        impl RLPEncode for StorageRootCodec<'_> {
            fn encode(&self, buf: &mut dyn BufMut) {
                let data = if *self.0 != *EMPTY_TRIE_HASH {
                    self.0.as_bytes()
                } else {
                    &[]
                };

                data.encode(buf);
            }
        }

        struct CodeHashCodec<'a>(&'a H256);
        impl RLPEncode for CodeHashCodec<'_> {
            fn encode(&self, buf: &mut dyn BufMut) {
                let data = if *self.0 != *EMPTY_KECCACK_HASH {
                    self.0.as_bytes()
                } else {
                    &[]
                };

                data.encode(buf);
            }
        }

        Encoder::new(buf)
            .encode_field(&self.0.nonce)
            .encode_field(&self.0.balance)
            .encode_field(&StorageRootCodec(&self.0.storage_root))
            .encode_field(&CodeHashCodec(&self.0.code_hash))
            .finish();
    }
}

impl RLPDecode for AccountStateSlimCodec {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        struct StorageRootCodec(H256);
        impl RLPDecode for StorageRootCodec {
            fn decode_unfinished(mut rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
                let value = match rlp.split_off_first() {
                    Some(0x80) => *EMPTY_TRIE_HASH,
                    Some(0xA0) => {
                        let data;
                        (data, rlp) = rlp
                            .split_first_chunk::<32>()
                            .ok_or(RLPDecodeError::InvalidLength)?;
                        H256(*data)
                    }
                    _ => return Err(RLPDecodeError::InvalidLength),
                };

                Ok((Self(value), rlp))
            }
        }

        struct CodeHashCodec(H256);
        impl RLPDecode for CodeHashCodec {
            fn decode_unfinished(mut rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
                let value = match rlp.split_off_first() {
                    Some(0x80) => *EMPTY_KECCACK_HASH,
                    Some(0xA0) => {
                        let data;
                        (data, rlp) = rlp
                            .split_first_chunk::<32>()
                            .ok_or(RLPDecodeError::InvalidLength)?;
                        H256(*data)
                    }
                    _ => return Err(RLPDecodeError::InvalidLength),
                };

                Ok((Self(value), rlp))
            }
        }

        let decoder = Decoder::new(rlp)?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let (balance, decoder) = decoder.decode_field("balance")?;
        let (StorageRootCodec(storage_root), decoder) = decoder.decode_field("storage_root")?;
        let (CodeHashCodec(code_hash), decoder) = decoder.decode_field("code_hash")?;

        Ok((
            Self(AccountState {
                nonce,
                balance,
                storage_root,
                code_hash,
            }),
            decoder.finish()?,
        ))
    }
}

pub fn compute_storage_root(storage: &BTreeMap<U256, U256>) -> H256 {
    let iter = storage.iter().filter_map(|(k, v)| {
        (!v.is_zero()).then_some((keccak_hash(k.to_big_endian()).to_vec(), v.encode_to_vec()))
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
