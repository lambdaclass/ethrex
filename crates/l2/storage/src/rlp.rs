// TODO: Remove this after `l2` feature is gone.
#![allow(dead_code)]

use std::{fmt::Debug, marker::PhantomData};

use ethrex_common::{H256, types::BlockNumber};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

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
