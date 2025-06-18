use libmdbx::orm::{Decodable, Encodable};

pub struct FlatTablesBlockMetadataKey();

impl Encodable for FlatTablesBlockMetadataKey {
    type Encoded = [u8; 0];
    fn encode(self) -> Self::Encoded {
        []
    }
}
impl Decodable for FlatTablesBlockMetadataKey {
    fn decode(_b: &[u8]) -> anyhow::Result<Self> {
        Ok(FlatTablesBlockMetadataKey {})
    }
}
