use ethereum_types::U256;
use libmdbx::orm::{Decodable, Encodable};

#[derive(Clone)]
pub struct AccountStorageValueBytes(pub [u8; 32]);

impl Encodable for AccountStorageValueBytes {
    type Encoded = [u8; 32];

    fn encode(self) -> Self::Encoded {
        self.0
    }
}

impl Decodable for AccountStorageValueBytes {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        Ok(AccountStorageValueBytes(b.try_into()?))
    }
}

impl From<U256> for AccountStorageValueBytes {
    fn from(value: U256) -> Self {
        AccountStorageValueBytes(value.to_big_endian())
    }
}

impl From<AccountStorageValueBytes> for U256 {
    fn from(value: AccountStorageValueBytes) -> Self {
        U256::from_big_endian(&value.0)
    }
}
