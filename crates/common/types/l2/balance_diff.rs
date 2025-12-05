use bytes::BufMut;
use ethereum_types::{Address, U256};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValuePerToken {
    pub token_l1: Address,
    pub token_l2: Address,
    pub other_chain_token_l2: Address,
    pub value: U256,
}

/// Represents the amount of balance to transfer to the bridge contract for a specific chain.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BalanceDiff {
    pub chain_id: U256,
    pub value_per_token: Vec<ValuePerToken>,
}

impl RLPEncode for BalanceDiff {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.chain_id.encode(buf);
        self.value_per_token.encode(buf);
    }
}

impl RLPDecode for BalanceDiff {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (chain_id, rlp) = U256::decode_unfinished(rlp)?;
        let (value_per_token, rlp) = Vec::<ValuePerToken>::decode_unfinished(rlp)?;
        Ok((
            BalanceDiff {
                chain_id,
                value_per_token,
            },
            rlp,
        ))
    }
}

impl RLPEncode for ValuePerToken {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.token_l1.encode(buf);
        self.token_l2.encode(buf);
        self.other_chain_token_l2.encode(buf);
        self.value.encode(buf);
    }
}

impl RLPDecode for ValuePerToken {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (token_l1, rlp) = Address::decode_unfinished(rlp)?;
        let (token_l2, rlp) = Address::decode_unfinished(rlp)?;
        let (other_chain_token_l2, rlp) = Address::decode_unfinished(rlp)?;
        let (value, rlp) = U256::decode_unfinished(rlp)?;
        Ok((
            ValuePerToken {
                token_l1,
                token_l2,
                other_chain_token_l2,
                value,
            },
            rlp,
        ))
    }
}
