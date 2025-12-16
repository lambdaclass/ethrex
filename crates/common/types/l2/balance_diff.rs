use bytes::BufMut;
use ethereum_types::{H256, U256};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use serde::{Deserialize, Serialize};

/// Represents the amount of balance to transfer to the bridge contract for a specific chain.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BalanceDiff {
    pub chain_id: U256,
    pub value: U256,
    pub message_hashes: Vec<H256>,
}

impl RLPEncode for BalanceDiff {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.chain_id.encode(buf);
        self.value.encode(buf);
        self.message_hashes.encode(buf);
    }
}

impl RLPDecode for BalanceDiff {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (chain_id, rlp) = U256::decode_unfinished(rlp)?;
        let (value, rlp) = U256::decode_unfinished(rlp)?;
        let (message_hashes, rlp) = Vec::<H256>::decode_unfinished(rlp)?;
        Ok((
            BalanceDiff {
                chain_id,
                value,
                message_hashes,
            },
            rlp,
        ))
    }
}
