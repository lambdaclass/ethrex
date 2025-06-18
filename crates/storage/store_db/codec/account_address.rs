use ethrex_common::H160;
use libmdbx::orm::{Decodable, Encodable};

#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct AccountAddress(pub H160);

impl From<H160> for AccountAddress {
    fn from(value: H160) -> Self {
        Self(value)
    }
}

impl Encodable for AccountAddress {
    type Encoded = [u8; 20];

    fn encode(self) -> Self::Encoded {
        self.0.0
    }
}

impl Decodable for AccountAddress {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        Ok(AccountAddress(H160(b.try_into()?)))
    }
}
