use std::collections::HashMap;

use bytes::Bytes;
use ethereum_types::{H256, U256};

use crate::rlp::{encode::RLPEncode, structs::Encoder};

use super::GenesisAccount;

#[allow(unused)]
#[derive(Debug, PartialEq)]
pub struct Account {
    pub info: AccountInfo,
    pub code: Bytes,
    pub storage: HashMap<H256, H256>,
}

#[derive(Debug, PartialEq)]
pub struct AccountInfo {
    pub code_hash: H256,
    pub balance: U256,
    pub nonce: u64,
}

impl From<GenesisAccount> for Account {
    fn from(genesis: GenesisAccount) -> Self {
        Self {
            info: AccountInfo {
                code_hash: code_hash(&genesis.code),
                balance: genesis.balance,
                nonce: genesis.nonce,
            },
            code: genesis.code,
            storage: genesis.storage,
        }
    }
}

fn code_hash(code: &Bytes) -> H256 {
    keccak_hash::keccak(code.as_ref())
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
