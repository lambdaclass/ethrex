use bytes::Bytes;
use ethereum_types::{Address, U256};
use ethrex_rlp::{decode::RLPDecode, structs::Decoder};
use serde::{Deserialize, Serialize};

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
