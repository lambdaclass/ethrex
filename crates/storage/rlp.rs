use std::fmt::Debug;
use std::marker::PhantomData;

use ethrex_common::{
    H256,
    types::{Block, BlockBody, BlockHeader, Receipt},
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

// Account types
pub type AccountCodeHashRLP = Rlp<H256>;

// Block types
pub type BlockHeaderRLP = Rlp<BlockHeader>;
pub type BlockBodyRLP = Rlp<BlockBody>;
pub type BlockRLP = Rlp<Block>;

// Receipt types
#[allow(unused)]
pub type ReceiptRLP = Rlp<Receipt>;

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

    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}
