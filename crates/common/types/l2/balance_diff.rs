use ethereum_types::{Address, H256, U256};
use librlp::{RlpDecode, RlpEncode, RlpError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetDiff {
    pub token_l1: Address,
    pub token_src_l2: Address,
    pub token_dst_l2: Address,
    pub value: U256,
}

/// Represents the amount of balance to transfer to the bridge contract for a specific chain.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BalanceDiff {
    pub chain_id: U256,
    pub value: U256,
    pub value_per_token: Vec<AssetDiff>,
    pub message_hashes: Vec<H256>,
}

impl RlpEncode for BalanceDiff {
    fn encode(&self, buf: &mut librlp::RlpBuf) {
        self.chain_id.encode(buf);
        self.value.encode(buf);
        librlp::encode_list(&self.value_per_token, buf);
        librlp::encode_list(&self.message_hashes, buf);
    }

    fn encoded_length(&self) -> usize {
        self.chain_id.encoded_length()
            + self.value.encoded_length()
            + crate::constants::vec_encoded_length(&self.value_per_token)
            + crate::constants::vec_encoded_length(&self.message_hashes)
    }
}

impl RlpDecode for BalanceDiff {
    fn decode(buf: &mut &[u8]) -> Result<Self, RlpError> {
        let chain_id = U256::decode(buf)?;
        let value = U256::decode(buf)?;
        let value_per_token = librlp::decode_list(buf)?;
        let message_hashes = librlp::decode_list(buf)?;
        Ok(BalanceDiff {
            chain_id,
            value,
            value_per_token,
            message_hashes,
        })
    }
}

impl RlpEncode for AssetDiff {
    fn encode(&self, buf: &mut librlp::RlpBuf) {
        self.token_l1.encode(buf);
        self.token_src_l2.encode(buf);
        self.token_dst_l2.encode(buf);
        self.value.encode(buf);
    }

    fn encoded_length(&self) -> usize {
        self.token_l1.encoded_length()
            + self.token_src_l2.encoded_length()
            + self.token_dst_l2.encoded_length()
            + self.value.encoded_length()
    }
}

impl RlpDecode for AssetDiff {
    fn decode(buf: &mut &[u8]) -> Result<Self, RlpError> {
        let token_l1 = Address::decode(buf)?;
        let token_src_l2 = Address::decode(buf)?;
        let token_dst_l2 = Address::decode(buf)?;
        let value = U256::decode(buf)?;
        Ok(AssetDiff {
            token_l1,
            token_src_l2,
            token_dst_l2,
            value,
        })
    }
}
