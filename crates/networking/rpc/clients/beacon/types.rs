use ethrex_common::{H256, U256};
use serde::Deserialize;
use sha2::Digest;

/// `data` structure of `/eth/v2/beacon/blocks/{block_id}` endpoint's response
#[derive(Deserialize, Debug)]
pub struct GetBlockResponseData {
    pub message: GetBlockResponseMessage,
    // 96 bytes hex string
    #[serde(rename = "signature", with = "ethrex_common::serde_utils::bytes")]
    _signature: bytes::Bytes,
}

/// `data.message` structure of `/eth/v2/beacon/blocks/{block_id}` endpoint's response
// Actual response has many more fields, but we only care about `slot` for now
#[derive(Deserialize, Debug)]
pub struct GetBlockResponseMessage {
    #[serde(deserialize_with = "ethrex_common::serde_utils::u256::deser_dec_str")]
    pub slot: U256,
}

/// Each element of `data` array of `/eth/v1/beacon/blob_sidecars/{block_id}` endpoint's response
// Actual response has many more fields, but we only care about these for now
#[derive(Deserialize, Debug)]
pub struct BlobSidecar {
    #[serde(deserialize_with = "ethrex_common::serde_utils::u64::deser_dec_str")]
    pub index: u64,
    #[serde(with = "ethrex_common::serde_utils::bytes")]
    pub blob: bytes::Bytes,
    #[serde(with = "ethrex_common::serde_utils::bytes")]
    pub kzg_commitment: bytes::Bytes,
}

impl BlobSidecar {
    pub fn versioned_hash(&self) -> H256 {
        let mut hasher = sha2::Sha256::new();
        hasher.update(&self.kzg_commitment);

        let hash = &mut hasher.finalize();
        hash[0] = 0x01;

        H256::from_slice(hash)
    }
}
