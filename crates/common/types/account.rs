use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

use bytes::{BufMut, Bytes};
use ethereum_types::{H256, U256};
use ethrex_crypto::{Crypto, NativeCrypto};
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
use crate::constants::{EMPTY_KECCAK_HASH, EMPTY_TRIE_HASH};

/// Shared empty jump-target table. `Code::default()` and any bytecode without a
/// `JUMPDEST` clone this (a refcount bump) instead of allocating a fresh empty
/// `Arc` header each time. This matters because the per-tx `Code::default()`
/// placeholder and every EOA / empty-code load would otherwise each allocate.
static EMPTY_JUMP_TARGETS: LazyLock<Arc<[u32]>> = LazyLock::new(|| Arc::from(Vec::new()));

/// Trailing STOP bytes appended to every bytecode so the dispatch loop can read
/// the next opcode without a bounds check. 33 is the widest single-opcode advance
/// (PUSH32: 1 opcode byte + 32 immediate bytes), so `pc` can never step past the
/// padding regardless of which opcode sits at the last real byte.
pub const BYTECODE_PADDING: usize = 33;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Code {
    // hash is only used for bytecodes stored in the DB, either for reading it from the DB
    // or with the CODEHASH opcode, which needs an account address as argument and
    // thus only accessed persisted bytecodes.
    // We use a bogus H256::zero() value for initcodes as there is no way for the VM or
    // endpoints to access that hash, saving one expensive Keccak hash.
    pub hash: H256,
    /// bytecode padded with 33 zeroes (STOP opcodes, due to PUSH32) to avoid checks on the hot path.
    bytecode: Bytes,
    /// The real bytecode length, needed for some opcodes, `bytecode` is padded with 33 STOPs to avoid checked adds on hot loop.
    bytecode_len: usize,
    // `Arc<[u32]>` so cloning `Code` (hot: every message-call resolves and clones
    // the callee's code) is a refcount bump instead of deep-copying the table.
    // Serializes via serde's `rc` feature (enabled workspace-wide).
    // The valid addresses are 32-bit because, despite EIP-3860 restricting initcode size,
    // this does not apply to previous forks. This is tested in the EEST tests, which would
    // panic in debug mode.
    pub jump_targets: Arc<[u32]>,
}

impl Code {
    // SAFETY: hash will be stored as-is, so it either needs to match
    // the real code hash (i.e. it was precomputed and we're reusing)
    // or never be read (e.g. for initcode).
    //
    // `code` is the logical, unpadded bytecode; `BYTECODE_PADDING` STOP bytes are
    // appended internally by `from_parts_unchecked`.
    pub fn from_bytecode_unchecked(code: Bytes, hash: H256) -> Self {
        let jump_targets = Self::compute_jump_targets(&code);
        Self::from_parts_unchecked(hash, &code, jump_targets)
    }

    /// `code` is the logical, unpadded bytecode; `BYTECODE_PADDING` STOP bytes are
    /// appended internally by `from_parts_unchecked`.
    pub fn from_bytecode(code: Bytes, crypto: &dyn Crypto) -> Self {
        let jump_targets = Self::compute_jump_targets(&code);
        let hash = H256(crypto.keccak256(code.as_ref()));
        Self::from_parts_unchecked(hash, &code, jump_targets)
    }

    /// Builds a `Code` from precomputed parts. The caller must guarantee `hash`
    /// and `jump_targets` correspond to `code`; neither is recomputed or validated.
    ///
    /// `code` is the logical, unpadded bytecode: this function appends
    /// `BYTECODE_PADDING` STOP bytes and records the original length in
    /// `bytecode_len`. Never pass a pre-padded buffer, or the logical length and
    /// every `JUMPDEST`/`PUSH` offset derived from it would be wrong.
    pub fn from_parts_unchecked(hash: H256, code: &[u8], jump_targets: Arc<[u32]>) -> Self {
        let bytecode_len = code.len();
        let mut padded_code = Vec::with_capacity(bytecode_len + BYTECODE_PADDING);
        padded_code.extend_from_slice(code);
        padded_code.extend_from_slice(&[0u8; BYTECODE_PADDING]);
        Self {
            hash,
            bytecode: Bytes::from_owner(padded_code),
            bytecode_len,
            jump_targets,
        }
    }

    fn compute_jump_targets(code: &[u8]) -> Arc<[u32]> {
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
        // Share the single empty table for jumpless bytecode (very common: EOAs,
        // tiny contracts) so we don't allocate an `Arc` header for an empty slice.
        if targets.is_empty() {
            EMPTY_JUMP_TARGETS.clone()
        } else {
            Arc::from(targets)
        }
    }

    #[inline]
    pub fn code(&self) -> &[u8] {
        self.bytecode.get(..self.bytecode_len).unwrap_or_default()
    }

    #[inline]
    pub fn code_bytes(&self) -> Bytes {
        self.bytecode.slice(..self.bytecode_len)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.bytecode_len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bytecode_len == 0
    }

    /// Returns the padded bytecode buffer (real code + [`BYTECODE_PADDING`] trailing
    /// STOPs) used by the opcode dispatch loop to read opcodes without bounds checks.
    /// Use [`Code::code`] for the real, unpadded bytecode.
    #[inline]
    pub fn dispatch_buf(&self) -> &[u8] {
        &self.bytecode
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
        let vec_size = size_of::<Arc<[u32]>>() + self.jump_targets.len() * size_of::<u32>();
        hash_size + bytes_size + vec_size
    }
}

/// Serde shadow for [`Code`]. Stores the *logical* (unpadded) bytecode so the
/// padding is never part of the serialized form. Deserialization re-pads through
/// [`Code::from_parts_unchecked`], which keeps the dispatch-loop invariant (every
/// `Code` is padded with [`BYTECODE_PADDING`] trailing STOPs) sound regardless of
/// where the bytes came from. Deserializing the padded buffer directly would
/// otherwise let unpadded input through and cause OOB reads during execution.
#[derive(Serialize, Deserialize)]
struct CodeSerde {
    hash: H256,
    code: Bytes,
    jump_targets: Arc<[u32]>,
}

impl Serialize for Code {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        CodeSerde {
            hash: self.hash,
            code: self.code_bytes(),
            jump_targets: self.jump_targets.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Code {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let CodeSerde {
            hash,
            code,
            jump_targets,
        } = CodeSerde::deserialize(deserializer)?;
        Ok(Self::from_parts_unchecked(hash, &code, jump_targets))
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
            code_hash: *EMPTY_KECCAK_HASH,
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
            code_hash: *EMPTY_KECCAK_HASH,
        }
    }
}

impl Default for Code {
    fn default() -> Self {
        Self {
            bytecode: Bytes::from_static(&[0u8; BYTECODE_PADDING]),
            bytecode_len: 0,
            hash: *EMPTY_KECCAK_HASH,
            jump_targets: EMPTY_JUMP_TARGETS.clone(),
        }
    }
}

impl From<GenesisAccount> for Account {
    fn from(genesis: GenesisAccount) -> Self {
        let code = Code::from_bytecode(genesis.code, &NativeCrypto);
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

pub fn code_hash(code: &Bytes, crypto: &dyn Crypto) -> H256 {
    H256(crypto.keccak256(code.as_ref()))
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
                let data = if *self.0 != *EMPTY_KECCAK_HASH {
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
                    Some(0x80) => *EMPTY_KECCAK_HASH,
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

pub fn compute_storage_root(storage: &BTreeMap<U256, U256>, crypto: &dyn Crypto) -> H256 {
    let iter = storage.iter().filter_map(|(k, v)| {
        (!v.is_zero()).then_some((
            crypto.keccak256(&k.to_big_endian()).to_vec(),
            v.encode_to_vec(),
        ))
    });
    Trie::compute_hash_from_unsorted_iter(iter, crypto)
}

impl From<&GenesisAccount> for AccountState {
    fn from(value: &GenesisAccount) -> Self {
        AccountState {
            nonce: value.nonce,
            balance: value.balance,
            storage_root: compute_storage_root(&value.storage, &NativeCrypto),
            code_hash: code_hash(&value.code, &NativeCrypto),
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
        self.balance.is_zero() && self.nonce == 0 && self.code_hash == *EMPTY_KECCAK_HASH
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_code_hash() {
        let empty_code = Bytes::new();
        let hash = code_hash(&empty_code, &NativeCrypto);
        assert_eq!(
            hash,
            H256::from_str("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470")
                .unwrap()
        )
    }
}
