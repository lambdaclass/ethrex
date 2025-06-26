// TODO: Remove this after `l2` feature is gone.
#![allow(dead_code)]

#[cfg(feature = "redb")]
use std::any::type_name;
use std::{fmt::Debug, marker::PhantomData};

use ethrex_common::{H256, types::BlockNumber};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
#[cfg(feature = "libmdbx")]
use libmdbx::orm::{Decodable, Encodable};
#[cfg(feature = "redb")]
use redb::TypeName;

pub type MessageHashesRLP = Rlp<Vec<H256>>;
pub type BlockNumbersRLP = Rlp<Vec<BlockNumber>>;
pub type OperationsCountRLP = Rlp<Vec<u64>>;

#[derive(Clone, Debug)]
pub struct Rlp<T>(Vec<u8>, PhantomData<T>);

impl<T: RLPEncode> From<T> for Rlp<T> {
    fn from(value: T) -> Self {
        let mut buf = Vec::new();
        RLPEncode::encode(&value, &mut buf);
        Self(buf, Default::default())
    }
}

impl<T> Rlp<T> {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes, Default::default())
    }
}

#[allow(clippy::unwrap_used)]
impl<T: RLPDecode> Rlp<T> {
    pub fn to(&self) -> T {
        T::decode(&self.0).unwrap()
    }
}

#[cfg(feature = "redb")]
impl<T: Send + Sync + Debug> redb::Value for Rlp<T> {
    type SelfType<'a>
        = Rlp<T>
    where
        Self: 'a;

    type AsBytes<'a>
        = Vec<u8>
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        Rlp(data.to_vec(), Default::default())
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        value.0.clone()
    }

    fn type_name() -> redb::TypeName {
        TypeName::new(&format!("RLP<{}>", type_name::<T>()))
    }
}

#[cfg(feature = "libmdbx")]
impl<T: Send + Sync> Decodable for Rlp<T> {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        Ok(Rlp(b.to_vec(), Default::default()))
    }
}

#[cfg(feature = "libmdbx")]
impl<T: Send + Sync> Encodable for Rlp<T> {
    type Encoded = Vec<u8>;

    fn encode(self) -> Self::Encoded {
        self.0
    }
}
