use ethereum_types::{H256, U256};
use ethrex_common::H160;
use libmdbx::orm::{Decodable, Encodable};

#[derive(Default)]
pub struct AccountStorageLogEntry(pub H160, pub H256, pub U256, pub U256);

// implemente Encode and Decode for StorageStateWriteLogVal
impl Encodable for AccountStorageLogEntry {
    type Encoded = [u8; 116];

    fn encode(self) -> Self::Encoded {
        let mut encoded = [0u8; 116];
        encoded[0..20].copy_from_slice(&self.0.0);
        encoded[20..52].copy_from_slice(&self.1.0);
        encoded[52..84].copy_from_slice(&self.2.to_big_endian());
        encoded[84..116].copy_from_slice(&self.3.to_big_endian());
        encoded
    }
}

impl Decodable for AccountStorageLogEntry {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        if b.len() < std::mem::size_of::<Self>() {
            anyhow::bail!("Invalid length for StorageStateWriteLogVal");
        }
        let addr = H160::from_slice(&b[0..20]);
        let slot = H256::from_slice(&b[20..52]);
        let old_value = U256::from_big_endian(&b[52..84]);
        let new_value = U256::from_big_endian(&b[84..116]);
        Ok(Self(addr, slot, old_value, new_value))
    }
}
