use std::fmt::Debug;
use std::marker::PhantomData;

use bytes::Bytes;
use ethrex_common::types::{Block, BlockBody, BlockHeader, Receipt, payload::PayloadBundle};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
#[cfg(feature = "libmdbx")]
use libmdbx::orm::{Decodable, Encodable};

// Account types
pub type AccountCodeRLP = Rlp<Bytes>;

// Block types
pub type BlockHeaderRLP = Rlp<BlockHeader>;
pub type BlockBodyRLP = Rlp<BlockBody>;
pub type BlockRLP = Rlp<Block>;
#[allow(unused)]
pub type ReceiptRLP = Rlp<Receipt>;

// Payload type
pub type PayloadBundleRLP = Rlp<PayloadBundle>;

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
    pub fn to(&self) -> Result<T, ethrex_rlp::error::RLPDecodeError> {
        T::decode(&self.0)
    }
}

impl<T> Rlp<T> {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes, Default::default())
    }

    pub fn bytes(&self) -> &Vec<u8> {
        &self.0
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
