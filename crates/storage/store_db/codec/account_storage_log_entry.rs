use ethereum_types::{H256, U256};
use ethrex_common::H160;
#[cfg(feature = "redb")]
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
#[cfg(feature = "libmdbx")]
use libmdbx::orm::{Decodable, Encodable};

#[cfg(feature = "libmdbx")]
const SIZE_OF_ACCOUNT_STORAGE_LOG_ENTRY: usize = 116;

#[derive(Debug, Default, Clone)]
pub struct AccountStorageLogEntry {
    pub address: H160,
    pub slot: H256,
    pub old_value: U256,
    pub new_value: U256,
}

// implemente Encode and Decode for StorageStateWriteLogVal
#[cfg(feature = "libmdbx")]
impl Encodable for AccountStorageLogEntry {
    type Encoded = [u8; SIZE_OF_ACCOUNT_STORAGE_LOG_ENTRY];

    fn encode(self) -> Self::Encoded {
        let mut encoded: Self::Encoded = std::array::from_fn(|_| 0);
        encoded[0..20].copy_from_slice(&self.address.0);
        encoded[20..52].copy_from_slice(&self.slot.0);
        encoded[52..84].copy_from_slice(&self.old_value.to_big_endian());
        encoded[84..116].copy_from_slice(&self.new_value.to_big_endian());
        encoded
    }
}

#[cfg(feature = "libmdbx")]
impl Decodable for AccountStorageLogEntry {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        let len = b.len();
        if len < SIZE_OF_ACCOUNT_STORAGE_LOG_ENTRY {
            anyhow::bail!(
                "Invalid length for StorageStateWriteLogEntry: {len} (expected {SIZE_OF_ACCOUNT_STORAGE_LOG_ENTRY})"
            );
        }
        let address = H160::from_slice(&b[0..20]);
        let slot = H256::from_slice(&b[20..52]);
        let old_value = U256::from_big_endian(&b[52..84]);
        let new_value = U256::from_big_endian(&b[84..116]);
        Ok(Self {
            address,
            slot,
            old_value,
            new_value,
        })
    }
}

#[cfg(feature = "redb")]
impl RLPEncode for AccountStorageLogEntry {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.address)
            .encode_field(&self.slot)
            .encode_field(&self.old_value)
            .encode_field(&self.new_value)
            .finish();
    }
}

#[cfg(feature = "redb")]
impl RLPDecode for AccountStorageLogEntry {
    fn decode_unfinished(rlp: &[u8]) -> Result<(AccountStorageLogEntry, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (address, decoder) = decoder.decode_field("address")?;
        let (slot, decoder) = decoder.decode_field("slot")?;
        let (old_value, decoder) = decoder.decode_field("old_value")?;
        let (new_value, decoder) = decoder.decode_field("new_value")?;
        let log_entry = AccountStorageLogEntry {
            address,
            slot,
            old_value,
            new_value,
        };
        Ok((log_entry, decoder.finish()?))
    }
}
