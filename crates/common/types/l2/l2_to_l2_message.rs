use bytes::Bytes;
use ethereum_types::{Address, H256, U256};
use ethrex_rlp::{decode::RLPDecode, structs::Decoder};
use serde::{Deserialize, Serialize};

use crate::utils::keccak;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Represents a message from the L2 to another L2
pub struct L2toL2Message {
    /// Chain id of the destination chain
    pub chain_id: U256,
    /// Address that originated the transaction
    pub from: Address,
    /// Address of the recipient in the destination chain
    pub to: Address,
    /// Amount of ETH to send to the recipient
    pub value: U256,
    /// Gas limit for the transaction execution in the destination chain
    pub gas_limit: U256,
    /// Calldata for the transaction in the destination chain
    pub data: Bytes,
}

impl RLPDecode for L2toL2Message {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;

        let (chain_id, decoder) = decoder.decode_field("chain_id")?;
        let (from, decoder) = decoder.decode_field("from")?;
        let (to, decoder) = decoder.decode_field("to")?;
        let (value, decoder) = decoder.decode_field("value")?;
        let (gas_limit, decoder) = decoder.decode_field("gas_limit")?;
        let (data, decoder) = decoder.decode_field("data")?;

        Ok((
            L2toL2Message {
                chain_id,
                from,
                to,
                value,
                gas_limit,
                data,
            },
            decoder.finish()?,
        ))
    }
}

impl L2toL2Message {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.chain_id.to_big_endian());
        bytes.extend_from_slice(&self.from.to_fixed_bytes());
        bytes.extend_from_slice(&self.to.to_fixed_bytes());
        bytes.extend_from_slice(&self.value.to_big_endian());
        bytes.extend_from_slice(&self.gas_limit.to_big_endian());
        bytes.extend_from_slice(&self.data);
        bytes
    }
}

pub fn get_l2_message_hash(msg: &L2toL2Message) -> H256 {
    keccak(msg.encode())
}
