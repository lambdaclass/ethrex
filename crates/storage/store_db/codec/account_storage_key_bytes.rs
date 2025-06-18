use ethereum_types::H256;
use libmdbx::orm::{Decodable, Encodable};

// Storage values are stored as bytes instead of using their rlp encoding
// As they are stored in a dupsort table, they need to have a fixed size, and encoding them doesn't preserve their size
#[derive(Clone)]
pub struct AccountStorageKeyBytes(pub [u8; 32]);

impl Encodable for AccountStorageKeyBytes {
    type Encoded = [u8; 32];

    fn encode(self) -> Self::Encoded {
        self.0
    }
}

impl Decodable for AccountStorageKeyBytes {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        Ok(AccountStorageKeyBytes(b.try_into()?))
    }
}

impl From<H256> for AccountStorageKeyBytes {
    fn from(value: H256) -> Self {
        AccountStorageKeyBytes(value.0)
    }
}

impl From<AccountStorageKeyBytes> for H256 {
    fn from(value: AccountStorageKeyBytes) -> Self {
        H256(value.0)
    }
}
