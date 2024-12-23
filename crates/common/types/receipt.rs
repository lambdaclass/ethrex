use bytes::Bytes;
use ethereum_types::{Address, Bloom, BloomInput, H256};
use ethrex_rlp::{
    decode::{get_rlp_bytes_item_payload, is_encoded_as_bytes, RLPDecode},
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use serde::{Deserialize, Serialize};

use crate::types::TxType;
pub type Index = u64;

/// Result of a transaction
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Receipt {
    pub tx_type: TxType,
    pub succeeded: bool,
    pub cumulative_gas_used: u64,
    pub bloom: Bloom,
    pub logs: Vec<Log>,
}

impl Receipt {
    pub fn new(tx_type: TxType, succeeded: bool, cumulative_gas_used: u64, logs: Vec<Log>) -> Self {
        Self {
            tx_type,
            succeeded,
            cumulative_gas_used,
            bloom: bloom_from_logs(&logs),
            logs,
        }
    }
    // By reading the typed transactions EIP, and some geth code:
    // - https://eips.ethereum.org/EIPS/eip-2718
    // - https://github.com/ethereum/go-ethereum/blob/330190e476e2a2de4aac712551629a4134f802d5/core/types/receipt.go#L143
    // We've noticed the are some subtleties around encoding receipts and transactions.
    // First, `encode_inner` will encode a receipt according
    // to the RLP of its fields, if typed, the RLP of the fields
    // is padded with the byte representing this type.
    // For P2P messages, receipts are re-encoded as bytes
    // (see the `encode` implementation for receipt).
    // For debug and computing receipt roots, the expected
    // RLP encodings are the ones returned by `encode_inner`.
    // On some documentations, this is also called the `consensus-encoding`
    // for a receipt.

    /// Encodes Receipts in the following formats:
    /// A) Legacy receipts: rlp(receipt)
    /// B) Non legacy receipts: tx_type | rlp(receipt).
    pub fn encode_inner(&self) -> Vec<u8> {
        let mut encode_buff = match self.tx_type {
            TxType::Legacy => {
                vec![]
            }
            _ => {
                vec![self.tx_type as u8]
            }
        };
        Encoder::new(&mut encode_buff)
            .encode_field(&self.succeeded)
            .encode_field(&self.cumulative_gas_used)
            .encode_field(&self.bloom)
            .encode_field(&self.logs)
            .finish();
        encode_buff
    }

    /// Decodes Receipts in the following formats:
    /// A) Legacy receipts: rlp(receipt)
    /// B) Non legacy receipts: tx_type | rlp(receipt).
    pub fn decode_inner(rlp: &[u8]) -> Result<Receipt, RLPDecodeError> {
        // Obtain TxType
        let (tx_type, rlp) = match rlp.first() {
            Some(tx_type) if *tx_type < 0x7f => {
                let tx_type = match tx_type {
                    0x0 => TxType::Legacy,
                    0x1 => TxType::EIP2930,
                    0x2 => TxType::EIP1559,
                    0x3 => TxType::EIP4844,
                    0x7e => TxType::Privileged,
                    ty => {
                        return Err(RLPDecodeError::Custom(format!(
                            "Invalid transaction type: {ty}"
                        )))
                    }
                };
                (tx_type, &rlp[1..])
            }
            _ => (TxType::Legacy, rlp),
        };
        let decoder = Decoder::new(rlp)?;
        let (succeeded, decoder) = decoder.decode_field("succeeded")?;
        let (cumulative_gas_used, decoder) = decoder.decode_field("cumulative_gas_used")?;
        let (bloom, decoder) = decoder.decode_field("bloom")?;
        let (logs, decoder) = decoder.decode_field("logs")?;
        decoder.finish()?;

        Ok(Receipt {
            tx_type,
            succeeded,
            cumulative_gas_used,
            bloom,
            logs,
        })
    }
}

fn bloom_from_logs(logs: &[Log]) -> Bloom {
    let mut bloom = Bloom::zero();
    for log in logs {
        bloom.accrue(BloomInput::Raw(log.address.as_ref()));
        for topic in log.topics.iter() {
            bloom.accrue(BloomInput::Raw(topic.as_ref()));
        }
    }
    bloom
}

impl RLPEncode for Receipt {
    /// Receipts can be encoded in the following formats:
    /// A) Legacy receipts: rlp(receipt)
    /// B) Non legacy receipts: rlp(Bytes(tx_type | rlp(receipt))).
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        match self.tx_type {
            TxType::Legacy => {
                let legacy_encoded = self.encode_inner();
                buf.put_slice(&legacy_encoded);
            }
            _ => {
                let typed_recepipt_encoded = self.encode_inner();
                let bytes = Bytes::from(typed_recepipt_encoded);
                bytes.encode(buf);
            }
        };
    }
}

impl RLPDecode for Receipt {
    /// Receipts can be encoded in the following formats:
    /// A) Legacy receipts: rlp(receipt)
    /// B) Non legacy receipts: rlp(Bytes(tx_type | rlp(receipt))).
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (tx_type, rlp) = if is_encoded_as_bytes(rlp)? {
            let payload = get_rlp_bytes_item_payload(rlp)?;
            let tx_type = match payload.first().ok_or(RLPDecodeError::InvalidLength)? {
                0x0 => TxType::Legacy,
                0x1 => TxType::EIP2930,
                0x2 => TxType::EIP1559,
                0x3 => TxType::EIP4844,
                0x7e => TxType::Privileged,
                ty => {
                    return Err(RLPDecodeError::Custom(format!(
                        "Invalid transaction type: {ty}"
                    )))
                }
            };
            (tx_type, &payload[1..])
        } else {
            (TxType::Legacy, rlp)
        };

        let decoder = Decoder::new(rlp)?;
        let (succeeded, decoder) = decoder.decode_field("succeeded")?;
        let (cumulative_gas_used, decoder) = decoder.decode_field("cumulative_gas_used")?;
        let (bloom, decoder) = decoder.decode_field("bloom")?;
        let (logs, decoder) = decoder.decode_field("logs")?;

        Ok((
            Receipt {
                tx_type,
                succeeded,
                cumulative_gas_used,
                bloom,
                logs,
            },
            decoder.finish()?,
        ))
    }
}

/// Data record produced during the execution of a transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Log {
    pub address: Address,
    pub topics: Vec<H256>,
    pub data: Bytes,
}

impl RLPEncode for Log {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.address)
            .encode_field(&self.topics)
            .encode_field(&self.data)
            .finish();
    }
}

impl RLPDecode for Log {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (address, decoder) = decoder.decode_field("address")?;
        let (topics, decoder) = decoder.decode_field("topics")?;
        let (data, decoder) = decoder.decode_field("data")?;
        let log = Log {
            address,
            topics,
            data,
        };
        Ok((log, decoder.finish()?))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_encode_decode_receipt_legacy() {
        let receipt = Receipt {
            tx_type: TxType::Legacy,
            succeeded: true,
            cumulative_gas_used: 1200,
            bloom: Bloom::random(),
            logs: vec![Log {
                address: Address::random(),
                topics: vec![],
                data: Bytes::from_static(b"foo"),
            }],
        };
        let encoded_receipt = receipt.encode_to_vec();
        assert_eq!(receipt, Receipt::decode(&encoded_receipt).unwrap())
    }

    #[test]
    fn test_encode_decode_receipt_non_legacy() {
        let receipt = Receipt {
            tx_type: TxType::EIP4844,
            succeeded: true,
            cumulative_gas_used: 1500,
            bloom: Bloom::random(),
            logs: vec![Log {
                address: Address::random(),
                topics: vec![],
                data: Bytes::from_static(b"bar"),
            }],
        };
        let encoded_receipt = receipt.encode_to_vec();
        assert_eq!(receipt, Receipt::decode(&encoded_receipt).unwrap())
    }

    #[test]
    fn test_encode_decode_inner_receipt_legacy() {
        let receipt = Receipt {
            tx_type: TxType::Legacy,
            succeeded: true,
            cumulative_gas_used: 1200,
            bloom: Bloom::random(),
            logs: vec![Log {
                address: Address::random(),
                topics: vec![],
                data: Bytes::from_static(b"foo"),
            }],
        };
        let encoded_receipt = receipt.encode_inner();
        assert_eq!(receipt, Receipt::decode_inner(&encoded_receipt).unwrap())
    }

    #[test]
    fn test_encode_decode_receipt_inner_non_legacy() {
        let receipt = Receipt {
            tx_type: TxType::EIP4844,
            succeeded: true,
            cumulative_gas_used: 1500,
            bloom: Bloom::random(),
            logs: vec![Log {
                address: Address::random(),
                topics: vec![],
                data: Bytes::from_static(b"bar"),
            }],
        };
        let encoded_receipt = receipt.encode_inner();
        assert_eq!(receipt, Receipt::decode_inner(&encoded_receipt).unwrap())
    }
}
