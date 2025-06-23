#[cfg(feature = "libmdbx")]
use libmdbx::orm::{Decodable, Encodable};

#[cfg(feature = "libmdbx")]
pub struct FlatTablesBlockMetadataKey();

#[cfg(feature = "libmdbx")]
impl Encodable for FlatTablesBlockMetadataKey {
    type Encoded = [u8; 0];
    fn encode(self) -> Self::Encoded {
        []
    }
}

#[cfg(feature = "libmdbx")]
impl Decodable for FlatTablesBlockMetadataKey {
    fn decode(_b: &[u8]) -> anyhow::Result<Self> {
        Ok(FlatTablesBlockMetadataKey {})
    }
}
