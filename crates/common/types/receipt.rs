use bytes::Bytes;
use ethereum_types::{Address, Bloom, BloomInput, H256};
use ethrex_crypto::keccak::keccak_hash;
use librlp::{Header, RlpDecode, RlpEncode, RlpError};
use serde::{Deserialize, Serialize};

use crate::types::TxType;
pub type Index = u64;

/// Result of a transaction
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Receipt {
    pub tx_type: TxType,
    pub succeeded: bool,
    /// Cumulative gas used by this and all previous transactions in the block.
    /// This is always post-refund gas.
    /// Note: Block-level gas accounting (pre-refund for EIP-7778) uses BlockExecutionResult::block_gas_used.
    pub cumulative_gas_used: u64,
    pub logs: Vec<Log>,
}

impl Receipt {
    pub fn new(tx_type: TxType, succeeded: bool, cumulative_gas_used: u64, logs: Vec<Log>) -> Self {
        Self {
            tx_type,
            succeeded,
            cumulative_gas_used,
            logs,
        }
    }

    pub fn encode_inner(&self) -> Vec<u8> {
        let mut buf = librlp::RlpBuf::new();
        let tx_type: u8 = self.tx_type as u8;
        buf.list(|buf| {
            tx_type.encode(buf);
            self.succeeded.encode(buf);
            self.cumulative_gas_used.encode(buf);
            librlp::encode_list(&self.logs, buf);
        });
        buf.finish()
    }

    pub fn encode_inner_with_bloom(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(512);
        if self.tx_type != TxType::Legacy {
            result.push(self.tx_type as u8);
        }
        let bloom = bloom_from_logs(&self.logs);
        let mut buf = librlp::RlpBuf::new();
        buf.list(|buf| {
            self.succeeded.encode(buf);
            self.cumulative_gas_used.encode(buf);
            bloom.encode(buf);
            librlp::encode_list(&self.logs, buf);
        });
        result.extend_from_slice(&buf.finish());
        result
    }
}

pub fn bloom_from_logs(logs: &[Log]) -> Bloom {
    let mut bloom = Bloom::zero();
    for log in logs {
        let address_hash = keccak_hash(log.address);
        bloom.accrue(BloomInput::Hash(&address_hash));
        for topic in log.topics.iter() {
            let topic_hash = keccak_hash(*topic);
            bloom.accrue(BloomInput::Hash(&topic_hash));
        }
    }
    bloom
}

impl RlpEncode for Receipt {
    fn encode(&self, buf: &mut librlp::RlpBuf) {
        let encoded_inner = self.encode_inner();
        buf.put_bytes(&encoded_inner);
    }

    fn encoded_length(&self) -> usize {
        self.encode_inner().len()
    }
}

impl RlpDecode for Receipt {
    fn decode(buf: &mut &[u8]) -> Result<Self, RlpError> {
        let header = Header::decode(buf)?;
        if !header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut payload = &buf[..header.payload_length];
        let tx_type: u8 = RlpDecode::decode(&mut payload)?;
        let succeeded = RlpDecode::decode(&mut payload)?;
        let cumulative_gas_used = RlpDecode::decode(&mut payload)?;
        let logs = librlp::decode_list(&mut payload)?;

        let Some(tx_type) = TxType::from_u8(tx_type) else {
            return Err(RlpError::Custom(
                "Invalid transaction type".into(),
            ));
        };

        *buf = &buf[header.payload_length..];
        Ok(Receipt {
            tx_type,
            succeeded,
            cumulative_gas_used,
            logs,
        })
    }
}

/// Result of a transaction
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReceiptWithBloom {
    pub tx_type: TxType,
    pub succeeded: bool,
    /// Cumulative gas used by this and all previous transactions in the block.
    /// This is always post-refund gas.
    /// Note: Block-level gas accounting (pre-refund for EIP-7778) uses BlockExecutionResult::block_gas_used.
    pub cumulative_gas_used: u64,
    pub bloom: Bloom,
    pub logs: Vec<Log>,
}

impl ReceiptWithBloom {
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
        let mut result = match self.tx_type {
            TxType::Legacy => vec![],
            _ => vec![self.tx_type as u8],
        };
        let mut buf = librlp::RlpBuf::new();
        buf.list(|buf| {
            self.succeeded.encode(buf);
            self.cumulative_gas_used.encode(buf);
            self.bloom.encode(buf);
            librlp::encode_list(&self.logs, buf);
        });
        result.extend_from_slice(&buf.finish());
        result
    }

    /// Decodes Receipts in the following formats:
    /// A) Legacy receipts: rlp(receipt)
    /// B) Non legacy receipts: tx_type | rlp(receipt).
    pub fn decode_inner(rlp: &[u8]) -> Result<Self, RlpError> {
        // Obtain TxType
        let (tx_type, rlp) = match rlp.first() {
            Some(tx_type) if *tx_type < 0x7f => {
                let tx_type = match tx_type {
                    0x0 => TxType::Legacy,
                    0x1 => TxType::EIP2930,
                    0x2 => TxType::EIP1559,
                    0x3 => TxType::EIP4844,
                    0x4 => TxType::EIP7702,
                    0x7d => TxType::FeeToken,
                    0x7e => TxType::Privileged,
                    ty => {
                        return Err(RlpError::Custom(format!(
                            "Invalid transaction type: {ty}"
                        )));
                    }
                };
                (tx_type, &rlp[1..])
            }
            _ => (TxType::Legacy, rlp),
        };
        let mut buf = rlp;
        let header = Header::decode(&mut buf)?;
        if !header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut payload = &buf[..header.payload_length];
        let succeeded = RlpDecode::decode(&mut payload)?;
        let cumulative_gas_used = RlpDecode::decode(&mut payload)?;
        let bloom = RlpDecode::decode(&mut payload)?;
        let logs = librlp::decode_list(&mut payload)?;

        Ok(Self {
            tx_type,
            succeeded,
            cumulative_gas_used,
            bloom,
            logs,
        })
    }
}

impl RlpEncode for ReceiptWithBloom {
    /// Receipts can be encoded in the following formats:
    /// A) Legacy receipts: rlp(receipt)
    /// B) Non legacy receipts: rlp(Bytes(tx_type | rlp(receipt))).
    fn encode(&self, buf: &mut librlp::RlpBuf) {
        match self.tx_type {
            TxType::Legacy => {
                let legacy_encoded = self.encode_inner();
                buf.put_bytes(&legacy_encoded);
            }
            _ => {
                let typed_receipt_encoded = self.encode_inner();
                let bytes = Bytes::from(typed_receipt_encoded);
                bytes.encode(buf);
            }
        };
    }

    fn encoded_length(&self) -> usize {
        match self.tx_type {
            TxType::Legacy => self.encode_inner().len(),
            _ => {
                let typed_receipt_encoded = self.encode_inner();
                Bytes::from(typed_receipt_encoded).encoded_length()
            }
        }
    }
}

impl RlpDecode for ReceiptWithBloom {
    /// Receipts can be encoded in the following formats:
    /// A) Legacy receipts: rlp(receipt)
    /// B) Non legacy receipts: rlp(Bytes(tx_type | rlp(receipt))).
    fn decode(buf: &mut &[u8]) -> Result<Self, RlpError> {
        // The minimum size for a ReceiptWithBloom is > 256 bytes (due to the Bloom type field)
        // meaning that it is safe to check for bytes prefix to differentiate between
        // legacy receipts and non-legacy receipt payloads
        let header = Header::decode(buf)?;
        let (tx_type, payload_buf) = if !header.list {
            // Non-legacy: this was encoded as bytes wrapping type + rlp(receipt)
            let payload = &buf[..header.payload_length];
            let tx_type = match payload.first().ok_or(RlpError::InputTooShort)? {
                0x0 => TxType::Legacy,
                0x1 => TxType::EIP2930,
                0x2 => TxType::EIP1559,
                0x3 => TxType::EIP4844,
                0x4 => TxType::EIP7702,
                0x7d => TxType::FeeToken,
                0x7e => TxType::Privileged,
                ty => {
                    return Err(RlpError::Custom(format!(
                        "Invalid transaction type: {ty}"
                    )));
                }
            };
            *buf = &buf[header.payload_length..];
            (tx_type, &payload[1..])
        } else {
            // Legacy: header was a list header, we proceed to decode from the list payload
            (TxType::Legacy, &buf[..header.payload_length])
        };

        let mut inner = payload_buf;
        if tx_type != TxType::Legacy {
            // For non-legacy, we need to decode the inner list header
            let inner_header = Header::decode(&mut inner)?;
            if !inner_header.list {
                return Err(RlpError::UnexpectedString);
            }
            inner = &inner[..inner_header.payload_length];
        }

        let succeeded = RlpDecode::decode(&mut inner)?;
        let cumulative_gas_used = RlpDecode::decode(&mut inner)?;
        let bloom = RlpDecode::decode(&mut inner)?;
        let logs = librlp::decode_list(&mut inner)?;

        if tx_type == TxType::Legacy {
            *buf = &buf[header.payload_length..];
        }

        Ok(ReceiptWithBloom {
            tx_type,
            succeeded,
            cumulative_gas_used,
            bloom,
            logs,
        })
    }
}

impl From<&Receipt> for ReceiptWithBloom {
    fn from(receipt: &Receipt) -> Self {
        Self {
            tx_type: receipt.tx_type,
            succeeded: receipt.succeeded,
            cumulative_gas_used: receipt.cumulative_gas_used,
            bloom: bloom_from_logs(&receipt.logs),
            logs: receipt.logs.clone(),
        }
    }
}

impl From<&ReceiptWithBloom> for Receipt {
    fn from(receipt: &ReceiptWithBloom) -> Self {
        Self {
            tx_type: receipt.tx_type,
            succeeded: receipt.succeeded,
            cumulative_gas_used: receipt.cumulative_gas_used,
            logs: receipt.logs.clone(),
        }
    }
}

/// Data record produced during the execution of a transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Log {
    pub address: Address,
    pub topics: Vec<H256>,
    pub data: Bytes,
}

impl RlpEncode for Log {
    fn encode(&self, buf: &mut librlp::RlpBuf) {
        buf.list(|buf| {
            self.address.encode(buf);
            librlp::encode_list(&self.topics, buf);
            self.data.encode(buf);
        });
    }

    fn encoded_length(&self) -> usize {
        crate::constants::list_encoded_length(
            self.address.encoded_length()
                + crate::constants::vec_encoded_length(&self.topics)
                + self.data.encoded_length(),
        )
    }
}

impl RlpDecode for Log {
    fn decode(buf: &mut &[u8]) -> Result<Self, RlpError> {
        let header = Header::decode(buf)?;
        if !header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut payload = &buf[..header.payload_length];
        let address = RlpDecode::decode(&mut payload)?;
        let topics = librlp::decode_list(&mut payload)?;
        let data = RlpDecode::decode(&mut payload)?;
        *buf = &buf[header.payload_length..];
        Ok(Log {
            address,
            topics,
            data,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn h256_from_hex(s: &str) -> H256 {
        H256::from_slice(&hex::decode(s).unwrap())
    }

    #[test]
    fn test_encode_decode_receipt_legacy() {
        let receipt = Receipt {
            tx_type: TxType::Legacy,
            succeeded: true,
            cumulative_gas_used: 1200,
            logs: vec![Log {
                address: Address::random(),
                topics: vec![],
                data: Bytes::from_static(b"foo"),
            }],
        };
        let encoded_receipt = receipt.to_rlp();
        assert_eq!(receipt, Receipt::decode(&mut encoded_receipt.as_slice()).unwrap())
    }

    #[test]
    fn test_encode_decode_receipt_non_legacy() {
        let receipt = Receipt {
            tx_type: TxType::EIP4844,
            succeeded: true,
            cumulative_gas_used: 1500,
            logs: vec![Log {
                address: Address::random(),
                topics: vec![],
                data: Bytes::from_static(b"bar"),
            }],
        };
        let encoded_receipt = receipt.to_rlp();
        assert_eq!(receipt, Receipt::decode(&mut encoded_receipt.as_slice()).unwrap())
    }

    #[test]
    fn test_encode_decode_inner_receipt_legacy() {
        let receipt = ReceiptWithBloom {
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
        assert_eq!(
            receipt,
            ReceiptWithBloom::decode_inner(&encoded_receipt).unwrap()
        )
    }

    #[test]
    fn test_encode_decode_receipt_inner_non_legacy() {
        let receipt = ReceiptWithBloom {
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
        assert_eq!(
            receipt,
            ReceiptWithBloom::decode_inner(&encoded_receipt).unwrap()
        )
    }

    #[test]
    fn test_encode_receipt_with_bloom() {
        let receipt = Receipt {
            tx_type: TxType::EIP1559,
            succeeded: true,
            cumulative_gas_used: 1500,
            logs: vec![Log {
                address: Address::random(),
                topics: vec![
                    h256_from_hex(
                        "e70c0d1060ffbafc84e0e18d028245de3deeb0f41ecbade6562fa657d85ae945",
                    ),
                    h256_from_hex(
                        "e7e9cd61c8c6cb313324d785aa130fe50a7b9885e4d1d7700a327c5e9ae4e183",
                    ),
                    h256_from_hex(
                        "666d827b9db958c08f7186f127e3d9ea6a97288bcc4b527951ce493f6e2b76c4",
                    ),
                    h256_from_hex(
                        "28b4366544dccafad7b61138e9ada51706e85bb217a20cfa1c86e2648f8f369a",
                    ),
                    h256_from_hex(
                        "85cf9717f65c70d71cc6175f653512c13ce7b6a9bc5d9c2b9c49b2d2d6cb9536",
                    ),
                ],
                data: Bytes::from_static(b"bar"),
            }],
        };
        let encoded_receipt = receipt.encode_inner_with_bloom();

        let correct_bloom = {
            let mut bloom = Bloom::zero();
            for log in receipt.logs {
                bloom.accrue(BloomInput::Raw(log.address.as_ref()));
                for topic in log.topics.iter() {
                    bloom.accrue(BloomInput::Raw(topic.as_ref()));
                }
            }
            bloom
        };
        let receipt_with_bloom = ReceiptWithBloom::decode_inner(&encoded_receipt).unwrap();
        assert_eq!(receipt_with_bloom.bloom, correct_bloom);
    }
}
