use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::{BufMut, Bytes};
use ethrex_common::types::{Receipt, TxType};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};

#[derive(Debug, Clone)]
pub struct Receipts68 {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    pub receipts: Vec<Vec<Receipt>>,
}

/// Per-receipt wire wrapper for the eth/68 `Receipts` message.
///
/// eth/68 serves receipts *with* the bloom filter (unlike eth/69, which dropped
/// it). Each receipt is encoded as its EIP-2718 consensus byte form
/// (`tx_type || rlp(payload)` for typed, `rlp(payload)` for legacy) wrapped in
/// an RLP byte-string for typed receipts. This reproduces the historical
/// `ReceiptWithBloom` wire bytes, so an eth/68 peer reconstructs the same bytes
/// the receipts trie was built from.
#[derive(Debug, Clone)]
struct ReceiptItem68(Receipt);

impl RLPEncode for ReceiptItem68 {
    /// Mirrors `ReceiptWithBloom`'s wire encoding:
    /// A) Legacy receipts: `rlp(payload)` (raw list, no Bytes wrap).
    /// B) Non-legacy receipts: `rlp(Bytes(tx_type || rlp(payload)))`.
    fn encode(&self, buf: &mut dyn BufMut) {
        let inner = self.0.encode_inner_with_bloom(&ethrex_crypto::NativeCrypto);
        match self.0.tx_type {
            TxType::Legacy => buf.put_slice(&inner),
            _ => Bytes::from(inner).encode(buf),
        }
    }
}

impl RLPDecode for ReceiptItem68 {
    /// Inverse of [`ReceiptItem68`]'s encoding:
    /// A) Legacy receipts: `rlp(payload)`.
    /// B) Non-legacy receipts: `rlp(Bytes(tx_type || rlp(payload)))`.
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        // A non-legacy (typed) receipt is encoded as an RLP byte-string wrapping
        // `tx_type || rlp(payload)`; a legacy receipt is encoded as the raw RLP
        // list `rlp(payload)`. Distinguish by RLP item kind (string vs list).
        // NOTE: the `is_encoded_as_bytes` heuristic used elsewhere only matches
        // long strings (0xb8..=0xbf); it is unsafe here because frame receipts
        // carry no 256-byte bloom and can be short enough to use a short-string
        // prefix. Inspecting the item kind is robust for any size.
        let (is_list, payload, item_rest) = ethrex_rlp::decode::decode_rlp_item(rlp)?;
        if is_list {
            // Legacy: `decode_inner_with_bloom` expects the full RLP list
            // including its header, so decode from the original slice.
            let (receipt, rest) = Receipt::decode_inner_with_bloom(rlp)?;
            Ok((ReceiptItem68(receipt), rest))
        } else {
            // Typed: `payload` is exactly `tx_type || rlp(payload)`, bounded to
            // the byte-string's declared length; `item_rest` is the correct
            // remainder for the surrounding list decoder.
            let (receipt, inner_rest) = Receipt::decode_inner_with_bloom(payload)?;
            if !inner_rest.is_empty() {
                return Err(RLPDecodeError::Custom(
                    "trailing bytes in eth/68 receipt item".to_string(),
                ));
            }
            Ok((ReceiptItem68(receipt), item_rest))
        }
    }
}

impl Receipts68 {
    pub fn new(id: u64, receipts: Vec<Vec<Receipt>>) -> Self {
        Self { id, receipts }
    }

    pub fn get_receipts(&self) -> Vec<Vec<Receipt>> {
        self.receipts.clone()
    }

    pub fn get_id(&self) -> u64 {
        self.id
    }
}

impl RLPxMessage for Receipts68 {
    const CODE: u8 = 0x10;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let wire_receipts: Vec<Vec<ReceiptItem68>> = self
            .receipts
            .iter()
            .map(|block| block.iter().cloned().map(ReceiptItem68).collect())
            .collect();
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&wire_receipts)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (wire_receipts, _): (Vec<Vec<ReceiptItem68>>, _) = decoder.decode_field("receipts")?;
        let receipts = wire_receipts
            .into_iter()
            .map(|block| block.into_iter().map(|item| item.0).collect())
            .collect();

        Ok(Receipts68 { id, receipts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::{Log, ReceiptWithBloom};

    fn sample_nonframe_receipts() -> Vec<Vec<Receipt>> {
        vec![
            vec![
                Receipt {
                    tx_type: TxType::Legacy,
                    succeeded: true,
                    cumulative_gas_used: 21000,
                    logs: vec![Log {
                        address: Address::from_low_u64_be(0xaa),
                        topics: vec![],
                        data: CommonBytes::from_static(b"legacy"),
                    }],
                },
                Receipt {
                    tx_type: TxType::EIP1559,
                    succeeded: false,
                    cumulative_gas_used: 42000,
                    logs: vec![],
                },
            ],
            vec![Receipt {
                tx_type: TxType::EIP4844,
                succeeded: true,
                cumulative_gas_used: 100000,
                logs: vec![Log {
                    address: Address::from_low_u64_be(0xbb),
                    topics: vec![],
                    data: CommonBytes::from_static(b"blob"),
                }],
            }],
        ]
    }
    use ethrex_common::{Address, Bytes as CommonBytes};

    /// The eth/68 wire bytes for non-frame receipts must be IDENTICAL to the
    /// historical `Vec<Vec<ReceiptWithBloom>>` encoding, so existing eth/68 peers
    /// keep round-tripping. This builds the message the new way and the old way
    /// and asserts byte equality.
    #[test]
    fn nonframe_wire_bytes_match_legacy_receipt_with_bloom() {
        let receipts = sample_nonframe_receipts();

        // New path: Receipts68 stores Vec<Vec<Receipt>> and encodes via ReceiptItem68.
        let new_msg = Receipts68::new(7, receipts.clone());
        let mut new_buf = Vec::new();
        new_msg.encode(&mut new_buf).unwrap();

        // Old path: Vec<Vec<ReceiptWithBloom>> encoded directly, then snappy.
        let old_wire: Vec<Vec<ReceiptWithBloom>> = receipts
            .iter()
            .map(|block| block.iter().map(ReceiptWithBloom::from).collect())
            .collect();
        let mut old_encoded = Vec::new();
        Encoder::new(&mut old_encoded)
            .encode_field(&7u64)
            .encode_field(&old_wire)
            .finish();
        let old_buf = snappy_compress(old_encoded).unwrap();

        assert_eq!(new_buf, old_buf);
    }

    /// Full message round-trip for non-frame receipts.
    #[test]
    fn nonframe_message_roundtrips() {
        let receipts = sample_nonframe_receipts();
        let msg = Receipts68::new(3, receipts.clone());
        let mut buf = Vec::new();
        msg.encode(&mut buf).unwrap();
        let decoded = Receipts68::decode(&buf).unwrap();
        assert_eq!(decoded.get_id(), 3);
        assert_eq!(decoded.get_receipts(), receipts);
    }
}
