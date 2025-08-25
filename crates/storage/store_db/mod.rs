use std::ops::Deref;

use ethrex_common::{H256, U256};

pub mod in_memory;
#[cfg(feature = "libmdbx")]
pub mod libmdbx;

/// Fixed size encoding for storing in the database, this allows to store H256 and U256 values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bytes32(pub [u8; 32]);

impl Deref for Bytes32 {
    type Target = [u8; 32];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<H256> for Bytes32 {
    fn from(value: H256) -> Self {
        Self(value.0)
    }
}

impl From<U256> for Bytes32 {
    fn from(value: U256) -> Self {
        Self(value.to_little_endian())
    }
}

impl From<Bytes32> for U256 {
    fn from(value: Bytes32) -> Self {
        Self::from_little_endian(&value.0)
    }
}

impl From<Bytes32> for H256 {
    fn from(value: Bytes32) -> Self {
        Self(value.0)
    }
}
