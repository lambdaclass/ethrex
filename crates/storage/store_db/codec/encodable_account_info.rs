use ethereum_types::{H256, U256};
use ethrex_common::types::AccountInfo;
#[cfg(feature = "libmdbx")]
use libmdbx::orm::{Decodable, Encodable};

#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct EncodableAccountInfo(pub AccountInfo);

#[cfg(feature = "libmdbx")]
impl Encodable for EncodableAccountInfo {
    type Encoded = [u8; 72];
    fn encode(self) -> Self::Encoded {
        let mut encoded = [0u8; 72];
        encoded[0..32].copy_from_slice(&self.0.code_hash.to_fixed_bytes());
        encoded[32..64].copy_from_slice(&self.0.balance.to_big_endian());
        encoded[64..72].copy_from_slice(&self.0.nonce.to_be_bytes());
        encoded
    }
}

#[cfg(feature = "libmdbx")]
impl Decodable for EncodableAccountInfo {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        if b.len() < 72 {
            anyhow::bail!("too few bytes");
        }
        let mut nonce_bytes = [0u8; 8];
        nonce_bytes.copy_from_slice(&b[64..72]);
        Ok(Self(AccountInfo {
            code_hash: H256::from_slice(&b[0..32]),
            balance: U256::from_big_endian(&b[32..64]),
            nonce: u64::from_be_bytes(nonce_bytes),
        }))
    }
}

impl From<(H256, U256, u64)> for EncodableAccountInfo {
    fn from(value: (H256, U256, u64)) -> Self {
        Self(AccountInfo {
            code_hash: value.0,
            balance: value.1,
            nonce: value.2,
        })
    }
}
