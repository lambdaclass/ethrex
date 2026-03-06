use std::fmt::Debug;
use std::marker::PhantomData;

use ethrex_common::{
    H256,
    types::{Block, BlockBody, BlockHeader, Receipt},
};
use librlp::{RlpDecode, RlpEncode};

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

impl<T: RlpEncode> From<T> for Rlp<T> {
    fn from(value: T) -> Self {
        let buf = value.to_rlp();
        Self(buf, Default::default())
    }
}

impl<T: RlpDecode> Rlp<T> {
    pub fn to(&self) -> Result<T, librlp::RlpError> {
        T::decode(&mut self.0.as_slice())
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
