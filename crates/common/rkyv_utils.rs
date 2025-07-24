use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
};

use once_cell::sync::OnceCell;
use rkyv::{Archive, Deserialize, Serialize};

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(remote = ethereum_types::U256)]
pub struct U256Wrapper([u64; 4]);

impl From<U256Wrapper> for ethereum_types::U256 {
    fn from(value: U256Wrapper) -> Self {
        Self(value.0)
    }
}

#[derive(Archive, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
#[rkyv(remote = ethereum_types::H160)]
pub struct H160Wrapper([u8; 20]);

impl From<H160Wrapper> for ethereum_types::H160 {
    fn from(value: H160Wrapper) -> Self {
        Self(value.0)
    }
}

#[derive(Archive, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
#[rkyv(remote = ethereum_types::H256)]
pub struct H256Wrapper([u8; 32]);

impl From<H256Wrapper> for ethereum_types::H256 {
    fn from(value: H256Wrapper) -> Self {
        Self(value.0)
    }
}

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(remote = bytes::Bytes)]
pub struct BytesWrapper {
    #[rkyv(getter = bytes_to_vec)]
    bytes: Vec<u8>,
}

fn bytes_to_vec(bytes: &bytes::Bytes) -> Vec<u8> {
    bytes.to_vec()
}

impl From<BytesWrapper> for bytes::Bytes {
    fn from(value: BytesWrapper) -> Self {
        Self::copy_from_slice(&value.bytes)
    }
}

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(remote = ethereum_types::Bloom)]
pub struct BloomWrapper {
    #[rkyv(getter = bloom_to_bytes)]
    bloom_bytes: [u8; 256],
}

fn bloom_to_bytes(bloom: &ethereum_types::Bloom) -> [u8; 256] {
    bloom.0
}

impl From<BloomWrapper> for ethereum_types::Bloom {
    fn from(value: BloomWrapper) -> Self {
        Self::from_slice(&value.bloom_bytes)
    }
}

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(remote = OnceCell<ethereum_types::H256>)]
pub struct OnecCellBlockHashWrapper {
    #[rkyv(with = H256Wrapper, getter = h256_from_once_cell)]
    hash: ethereum_types::H256,
}

fn h256_from_once_cell(once: &OnceCell<ethereum_types::H256>) -> ethereum_types::H256 {
    // this is not currently working, always returns zero
    once.get().cloned().unwrap_or(ethereum_types::H256::zero())
}

impl From<OnecCellBlockHashWrapper> for OnceCell<ethereum_types::H256> {
    fn from(value: OnecCellBlockHashWrapper) -> Self {
        OnceCell::with_value(value.hash)
    }
}

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(remote = Option<ethereum_types::H256>)]
pub enum OptionH256Wrapper {
    Some(#[rkyv(with = H256Wrapper)] ethereum_types::H256),
    None,
}

impl From<OptionH256Wrapper> for Option<ethereum_types::H256> {
    fn from(value: OptionH256Wrapper) -> Self {
        if let OptionH256Wrapper::Some(x) = value {
            Some(x)
        } else {
            None
        }
    }
}

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(remote = Option<HashMap<ethereum_types::H160, Vec<Vec<u8>>>>)]
pub enum OptionStorageWrapper {
    Some(
        #[rkyv(with = rkyv::with::MapKV<H160Wrapper, rkyv::with::AsBox>)]
        HashMap<ethereum_types::H160, Vec<Vec<u8>>>,
    ),
    None,
}

impl From<OptionStorageWrapper> for Option<HashMap<ethereum_types::H160, Vec<Vec<u8>>>> {
    fn from(value: OptionStorageWrapper) -> Self {
        if let OptionStorageWrapper::Some(x) = value {
            Some(x)
        } else {
            None
        }
    }
}

impl PartialEq for ArchivedH256Wrapper {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for ArchivedH256Wrapper {}

impl Hash for ArchivedH256Wrapper {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl PartialEq for ArchivedH160Wrapper {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for ArchivedH160Wrapper {}

impl Hash for ArchivedH160Wrapper {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}
