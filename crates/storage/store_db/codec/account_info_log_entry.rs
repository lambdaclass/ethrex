use ethrex_common::H160;
use ethrex_common::types::AccountInfo;
#[cfg(feature = "libmdbx")]
use libmdbx::orm::{Decodable, Encodable};

#[derive(Default)]
pub struct AccountInfoLogEntry {
    pub address: H160,
    pub info: AccountInfo,
    pub previous_info: AccountInfo,
}

const SIZE_OF_ACCOUNT_INFO_LOG_ENTRY: usize = std::mem::size_of::<AccountInfoLogEntry>();

#[cfg(feature = "libmdbx")]
impl Encodable for AccountInfoLogEntry {
    type Encoded = [u8; std::mem::size_of::<Self>()];
    fn encode(self) -> Self::Encoded {
        let mut encoded: Self::Encoded = std::array::from_fn(|_| 0);
        encoded[0..20].copy_from_slice(&self.address.0);
        encoded[20..52].copy_from_slice(&self.info.code_hash.0);
        encoded[52..60].copy_from_slice(&self.info.nonce.to_be_bytes());
        encoded[60..92].copy_from_slice(&self.info.balance.to_big_endian());
        encoded[92..124].copy_from_slice(&self.previous_info.code_hash.0);
        encoded[124..132].copy_from_slice(&self.previous_info.nonce.to_be_bytes());
        encoded[132..164].copy_from_slice(&self.previous_info.balance.to_big_endian());
        encoded
    }
}

#[cfg(feature = "libmdbx")]
impl Decodable for AccountInfoLogEntry {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        if b.len() != SIZE_OF_ACCOUNT_INFO_LOG_ENTRY {
            anyhow::bail!("Invalid length for AccountInfoLog");
        }
        let addr = H160::from_slice(&b[0..20]);
        let info_code_hash = ethereum_types::H256::from_slice(&b[20..52]);
        let info_nonce = Decodable::decode(&b[52..60])?;
        let info_balance = ethereum_types::U256::from_big_endian(&b[60..92]);
        let previous_info_code_hash = ethereum_types::H256::from_slice(&b[92..124]);
        let previous_info_nonce = Decodable::decode(&b[124..132])?;
        let previous_info_balance = ethereum_types::U256::from_big_endian(&b[132..164]);
        Ok(Self {
            address: addr,
            info: AccountInfo {
                code_hash: info_code_hash,
                nonce: info_nonce,
                balance: info_balance,
            },
            previous_info: AccountInfo {
                code_hash: previous_info_code_hash,
                nonce: previous_info_nonce,
                balance: previous_info_balance,
            },
        })
    }
}
