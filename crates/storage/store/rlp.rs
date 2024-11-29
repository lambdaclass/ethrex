use std::fmt::Debug;
use std::marker::PhantomData;

use bytes::Bytes;
use ethereum_types::U256;
use ethrex_core::{
    types::{Block, BlockBody, BlockHash, BlockHeader, Receipt},
    H256,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
#[cfg(feature = "libmdbx")]
use libmdbx::orm::{Decodable, Encodable};
#[cfg(feature = "redb")]
use redb::TypeName;
#[cfg(feature = "redb")]
use std::any::type_name;

// Account types
pub type AccountCodeHashRLP = Rlp<H256>;
pub type AccountCodeRLP = Rlp<Bytes>;

// Block types
pub type BlockHashRLP = Rlp<BlockHash>;
pub type BlockHeaderRLP = Rlp<BlockHeader>;
pub type BlockBodyRLP = Rlp<BlockBody>;
pub type BlockRLP = Rlp<Block>;
// TODO (#307): Remove TotalDifficulty.
pub type BlockTotalDifficultyRLP = Rlp<U256>;

// Receipt types
pub type ReceiptRLP = Rlp<Receipt>;

// Transaction types
pub type TransactionHashRLP = Rlp<H256>;

// Wrapper for tuples. Used mostly for indexed keys.
pub type TupleRLP<A, B> = Rlp<(A, B)>;

#[derive(Clone, Debug)]
pub struct Rlp<T>(Vec<u8>, PhantomData<T>);

impl<T: RLPEncode> From<T> for Rlp<T> {
    fn from(value: T) -> Self {
        let mut buf = Vec::new();
        RLPEncode::encode(&value, &mut buf);
        Self(buf, Default::default())
    }
}

impl<T: RLPDecode> Rlp<T> {
    pub fn to(&self) -> T {
        T::decode(&self.0).unwrap()
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

#[cfg(feature = "redb")]
impl<T: Send + Sync + Debug> redb::Value for Rlp<T> {
    type SelfType<'a> = Rlp<T>
    where
        Self: 'a;

    type AsBytes<'a> = Vec<u8>
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

#[cfg(feature = "redb")]
impl<T: Send + Sync + Debug> redb::Key for Rlp<T> {
    fn compare(data1: &[u8], data2: &[u8]) -> std::cmp::Ordering {
        data1.cmp(data2)
    }
}
