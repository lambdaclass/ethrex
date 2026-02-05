use bytes::Bytes;
use ethereum_types::{Address, Bloom, BloomInput, H256};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::{
    decode::{RLPDecode, get_rlp_bytes_item_payload, is_encoded_as_bytes},
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
    /// Cumulative gas used by this and all previous transactions in the block.
    /// This is always post-refund gas.
    /// Note: Block-level gas accounting (pre-refund for EIP-7778) uses BlockExecutionResult::block_gas_used.
    pub cumulative_gas_used: u64,
    /// Gas spent after refunds (what the user actually pays).
    /// This is `None` for pre-EIP-7778 receipts.
    /// Post-EIP-7778 (Amsterdam+): This is included in RLP encoding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_spent: Option<u64>,
    pub logs: Vec<Log>,
}

impl Receipt {
    pub fn new(
        tx_type: TxType,
        succeeded: bool,
        cumulative_gas_used: u64,
        gas_spent: Option<u64>,
        logs: Vec<Log>,
    ) -> Self {
        Self {
            tx_type,
            succeeded,
            cumulative_gas_used,
            gas_spent,
            logs,
        }
    }

    pub fn encode_inner(&self) -> Vec<u8> {
        let mut encoded_data = vec![];
        let tx_type: u8 = self.tx_type as u8;
        let mut encoder = Encoder::new(&mut encoded_data)
            .encode_field(&tx_type)
            .encode_field(&self.succeeded)
            .encode_field(&self.cumulative_gas_used)
            .encode_field(&self.logs);

        // EIP-7778: Include gas_spent when present
        if let Some(gas_spent) = self.gas_spent {
            encoder = encoder.encode_field(&gas_spent);
        }
        encoder.finish();
        encoded_data
    }

    pub fn encode_inner_with_bloom(&self) -> Vec<u8> {
        // Bloom is already 256 bytes, so we preallocate at least that much plus some,
        // to avoid multiple small allocations.
        let mut encode_buf = Vec::with_capacity(512);
        if self.tx_type != TxType::Legacy {
            encode_buf.push(self.tx_type as u8);
        }
        let bloom = bloom_from_logs(&self.logs);
        let mut encoder = Encoder::new(&mut encode_buf)
            .encode_field(&self.succeeded)
            .encode_field(&self.cumulative_gas_used)
            .encode_field(&bloom)
            .encode_field(&self.logs);

        // EIP-7778: Include gas_spent when present
        if let Some(gas_spent) = self.gas_spent {
            encoder = encoder.encode_field(&gas_spent);
        }
        encoder.finish();
        encode_buf
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

impl RLPEncode for Receipt {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let encoded_inner = self.encode_inner();
        buf.put_slice(&encoded_inner);
    }
}

impl RLPDecode for Receipt {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        decode_receipt(rlp).map_err(|e| e.with_context("Receipt"))
    }
}

fn decode_receipt(rlp: &[u8]) -> Result<(Receipt, &[u8]), RLPDecodeError> {
    let decoder = Decoder::new(rlp)?;
    let (tx_type, decoder): (u8, _) = decoder.decode_field("tx-type")?;
    let (succeeded, decoder) = decoder.decode_field("succeeded")?;
    let (cumulative_gas_used, decoder) = decoder.decode_field("cumulative_gas_used")?;
    let (logs, decoder) = decoder.decode_field("logs")?;

    // EIP-7778: Try to decode optional gas_spent field
    let (gas_spent, decoder) = decoder.decode_optional_field();

    let Some(tx_type) = TxType::from_u8(tx_type) else {
        return Err(RLPDecodeError::Custom(
            "Invalid transaction type".to_string(),
        ));
    };

    Ok((
        Receipt {
            tx_type,
            succeeded,
            cumulative_gas_used,
            gas_spent,
            logs,
        },
        decoder.finish()?,
    ))
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
    /// Gas spent after refunds (what the user actually pays).
    /// This is `None` for pre-EIP-7778 receipts.
    /// Post-EIP-7778 (Amsterdam+): This is included in RLP encoding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_spent: Option<u64>,
}

impl ReceiptWithBloom {
    pub fn new(
        tx_type: TxType,
        succeeded: bool,
        cumulative_gas_used: u64,
        gas_spent: Option<u64>,
        logs: Vec<Log>,
    ) -> Self {
        Self {
            tx_type,
            succeeded,
            cumulative_gas_used,
            bloom: bloom_from_logs(&logs),
            logs,
            gas_spent,
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
        let mut encoder = Encoder::new(&mut encode_buff)
            .encode_field(&self.succeeded)
            .encode_field(&self.cumulative_gas_used)
            .encode_field(&self.bloom)
            .encode_field(&self.logs);

        // EIP-7778: Include gas_spent when present
        if let Some(gas_spent) = self.gas_spent {
            encoder = encoder.encode_field(&gas_spent);
        }
        encoder.finish();
        encode_buff
    }

    /// Decodes Receipts in the following formats:
    /// A) Legacy receipts: rlp(receipt)
    /// B) Non legacy receipts: tx_type | rlp(receipt).
    pub fn decode_inner(rlp: &[u8]) -> Result<Self, RLPDecodeError> {
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
                        return Err(RLPDecodeError::Custom(format!(
                            "Invalid transaction type: {ty}"
                        )));
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

        // EIP-7778: Try to decode optional gas_spent field
        let (gas_spent, decoder) = decoder.decode_optional_field();
        decoder.finish()?;

        Ok(Self {
            tx_type,
            succeeded,
            cumulative_gas_used,
            bloom,
            logs,
            gas_spent,
        })
    }
}

impl RLPEncode for ReceiptWithBloom {
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

impl RLPDecode for ReceiptWithBloom {
    /// Receipts can be encoded in the following formats:
    /// A) Legacy receipts: rlp(receipt)
    /// B) Non legacy receipts: rlp(Bytes(tx_type | rlp(receipt))).
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        decode_receipt_with_bloom(rlp).map_err(|e| e.with_context("ReceiptWithBloom"))
    }
}

fn decode_receipt_with_bloom(rlp: &[u8]) -> Result<(ReceiptWithBloom, &[u8]), RLPDecodeError> {
    // The minimum size for a ReceiptWithBloom is > 256 bytes (due to the Bloom type field) meaning that it is safe
    // to check for bytes prefix to diferenticate between legacy receipts and non-legacy receipt payloads
    let (tx_type, rlp) = if is_encoded_as_bytes(rlp)? {
        let payload = get_rlp_bytes_item_payload(rlp)?;
        let tx_type = match payload.first().ok_or(RLPDecodeError::invalid_length())? {
            0x0 => TxType::Legacy,
            0x1 => TxType::EIP2930,
            0x2 => TxType::EIP1559,
            0x3 => TxType::EIP4844,
            0x4 => TxType::EIP7702,
            0x7d => TxType::FeeToken,
            0x7e => TxType::Privileged,
            ty => {
                return Err(RLPDecodeError::Custom(format!(
                    "Invalid transaction type: {ty}"
                )));
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

    // EIP-7778: Try to decode optional gas_spent field
    let (gas_spent, decoder) = decoder.decode_optional_field();

    Ok((
        ReceiptWithBloom {
            tx_type,
            succeeded,
            cumulative_gas_used,
            bloom,
            logs,
            gas_spent,
        },
        decoder.finish()?,
    ))
}

impl From<&Receipt> for ReceiptWithBloom {
    fn from(receipt: &Receipt) -> Self {
        Self {
            tx_type: receipt.tx_type,
            succeeded: receipt.succeeded,
            cumulative_gas_used: receipt.cumulative_gas_used,
            bloom: bloom_from_logs(&receipt.logs),
            logs: receipt.logs.clone(),
            gas_spent: receipt.gas_spent,
        }
    }
}

impl From<&ReceiptWithBloom> for Receipt {
    fn from(receipt: &ReceiptWithBloom) -> Self {
        Self {
            tx_type: receipt.tx_type,
            succeeded: receipt.succeeded,
            cumulative_gas_used: receipt.cumulative_gas_used,
            gas_spent: receipt.gas_spent,
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

    fn h256_from_hex(s: &str) -> H256 {
        H256::from_slice(&hex::decode(s).unwrap())
    }

    #[test]
    fn test_encode_decode_receipt_legacy() {
        let receipt = Receipt {
            tx_type: TxType::Legacy,
            succeeded: true,
            cumulative_gas_used: 1200,
            gas_spent: None, // Pre-EIP-7778
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
            gas_spent: None, // Pre-EIP-7778
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
    fn test_encode_decode_receipt_with_gas_spent() {
        // EIP-7778: Test receipt with gas_spent field
        let receipt = Receipt {
            tx_type: TxType::EIP1559,
            succeeded: true,
            cumulative_gas_used: 1500,
            gas_spent: Some(1400), // Post-EIP-7778
            logs: vec![Log {
                address: Address::random(),
                topics: vec![],
                data: Bytes::from_static(b"baz"),
            }],
        };
        let encoded_receipt = receipt.encode_to_vec();
        assert_eq!(receipt, Receipt::decode(&encoded_receipt).unwrap())
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
            gas_spent: None, // Pre-EIP-7778
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
            gas_spent: None, // Pre-EIP-7778
        };
        let encoded_receipt = receipt.encode_inner();
        assert_eq!(
            receipt,
            ReceiptWithBloom::decode_inner(&encoded_receipt).unwrap()
        )
    }

    #[test]
    fn test_encode_decode_receipt_with_bloom_gas_spent() {
        // EIP-7778: Test receipt with bloom and gas_spent field
        let receipt = ReceiptWithBloom {
            tx_type: TxType::EIP1559,
            succeeded: true,
            cumulative_gas_used: 1500,
            bloom: Bloom::random(),
            logs: vec![Log {
                address: Address::random(),
                topics: vec![],
                data: Bytes::from_static(b"baz"),
            }],
            gas_spent: Some(1400), // Post-EIP-7778
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
            gas_spent: None, // Pre-EIP-7778
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
