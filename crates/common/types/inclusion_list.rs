//! EIP-7805 (FOCIL) `InclusionList` container — a list of EIP-2718-encoded
//! transactions that the Engine API carries between the consensus and
//! execution layers under `engine_getInclusionListV1`,
//! `engine_forkchoiceUpdatedV5`, and `engine_newPayloadV6`.

use bytes::Bytes;
use ethrex_rlp::error::RLPDecodeError;
use thiserror::Error;

use super::transaction::Transaction;

/// Per the FOCIL execution-apis spec: an inclusion list MUST NOT exceed
/// 8 KiB of total RLP-encoded transaction byte length.
pub const MAX_BYTES_PER_INCLUSION_LIST: usize = 8192;

/// A FOCIL inclusion list — RLP-decoded view over a `Vec<Bytes>` of
/// EIP-2718 transactions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InclusionList {
    pub transactions: Vec<Transaction>,
}

#[derive(Debug, Error)]
pub enum InclusionListError {
    #[error("inclusion list RLP decode failed: {0}")]
    Decode(#[from] RLPDecodeError),
    #[error(
        "inclusion list exceeds {MAX_BYTES_PER_INCLUSION_LIST} byte cap (total = {total} bytes)"
    )]
    TooLarge { total: usize },
}

impl InclusionList {
    /// Total RLP-encoded byte length of the inclusion list, computed as the
    /// sum of each transaction's EIP-2718 canonical encoding length. This is
    /// the byte count that must remain `<= MAX_BYTES_PER_INCLUSION_LIST`.
    pub fn encoded_byte_len(&self) -> usize {
        self.transactions
            .iter()
            .map(|tx| tx.encode_canonical_to_vec().len())
            .sum()
    }
}

impl TryFrom<Vec<Bytes>> for InclusionList {
    type Error = InclusionListError;

    fn try_from(encoded: Vec<Bytes>) -> Result<Self, Self::Error> {
        let total: usize = encoded.iter().map(|b| b.len()).sum();
        if total > MAX_BYTES_PER_INCLUSION_LIST {
            return Err(InclusionListError::TooLarge { total });
        }
        let transactions = encoded
            .iter()
            .map(|b| Transaction::decode_canonical(b.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { transactions })
    }
}

impl From<&InclusionList> for Vec<Bytes> {
    fn from(il: &InclusionList) -> Self {
        il.transactions
            .iter()
            .map(|tx| Bytes::from(tx.encode_canonical_to_vec()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::transaction::{EIP1559Transaction, LegacyTransaction, Transaction, TxKind};
    use ethereum_types::U256;
    use ethrex_rlp::encode::RLPEncode;
    use hex_literal::hex;

    fn sample_legacy_tx(nonce: u64) -> Transaction {
        Transaction::LegacyTransaction(LegacyTransaction {
            nonce,
            gas_price: U256::from(0x0a),
            gas: 0x05f5e100,
            to: TxKind::Call(hex!("1000000000000000000000000000000000000000").into()),
            value: U256::from(nonce),
            data: Default::default(),
            v: U256::from(0x1b),
            r: U256::from_big_endian(&hex!(
                "7e09e26678ed4fac08a249ebe8ed680bf9051a5e14ad223e4b2b9d26e0208f37"
            )),
            s: U256::from_big_endian(&hex!(
                "5f6e3f188e3e6eab7d7d3b6568f5eac7d687b08d307d3154ccd8c87b4630509b"
            )),
            ..Default::default()
        })
    }

    fn sample_eip1559_tx(nonce: u64) -> Transaction {
        Transaction::EIP1559Transaction(EIP1559Transaction {
            chain_id: 1,
            nonce,
            max_priority_fee_per_gas: 1_000_000_000,
            max_fee_per_gas: 2_000_000_000,
            gas_limit: 21_000,
            to: TxKind::Call(hex!("2000000000000000000000000000000000000000").into()),
            value: U256::from(42),
            data: Default::default(),
            access_list: vec![],
            signature_y_parity: false,
            signature_r: U256::from(1),
            signature_s: U256::from(2),
            ..Default::default()
        })
    }

    fn encode_canonical_bytes(tx: &Transaction) -> Bytes {
        Bytes::from(tx.encode_canonical_to_vec())
    }

    #[test]
    fn roundtrip_rlp_encode_decode() {
        let original = InclusionList {
            transactions: vec![
                sample_legacy_tx(0),
                sample_eip1559_tx(1),
                sample_legacy_tx(2),
            ],
        };

        let encoded: Vec<Bytes> = (&original).into();
        assert_eq!(encoded.len(), original.transactions.len());

        let decoded =
            InclusionList::try_from(encoded).expect("round-trip RLP decoding must succeed");
        // Compare by hash — `Transaction`'s PartialEq includes OnceCell caches
        // populated asymmetrically by encoding vs decoding paths.
        assert_eq!(decoded.transactions.len(), original.transactions.len());
        for (decoded_tx, original_tx) in decoded
            .transactions
            .iter()
            .zip(original.transactions.iter())
        {
            assert_eq!(decoded_tx.hash(), original_tx.hash());
        }
    }

    #[test]
    fn total_byte_length_computation() {
        let txs = vec![sample_legacy_tx(0), sample_eip1559_tx(1)];
        let il = InclusionList {
            transactions: txs.clone(),
        };

        let from_method = il.encoded_byte_len();
        let from_vec_bytes: usize = Vec::<Bytes>::from(&il).iter().map(|b| b.len()).sum();
        assert_eq!(from_method, from_vec_bytes);

        let from_canonical: usize = txs
            .iter()
            .map(|tx| tx.encode_canonical_to_vec().len())
            .sum();
        assert_eq!(from_method, from_canonical);
    }

    #[test]
    fn over_8kib_total_is_rejected() {
        // Build a Bytes payload that decodes to a tiny tx but whose total
        // length exceeds the 8 KiB cap. We pad with a second `Bytes` whose
        // length alone pushes the sum over.
        let one_tx_bytes = encode_canonical_bytes(&sample_legacy_tx(0));
        // Fabricate an oversized "filler" entry. It would not RLP-decode, but
        // the cap check runs before decoding, so this test exercises the
        // cap path explicitly.
        let oversize_filler = Bytes::from(vec![0u8; MAX_BYTES_PER_INCLUSION_LIST]);
        let encoded = vec![one_tx_bytes, oversize_filler];

        let total: usize = encoded.iter().map(|b| b.len()).sum();
        assert!(total > MAX_BYTES_PER_INCLUSION_LIST);

        let err = InclusionList::try_from(encoded)
            .expect_err("over-cap input must be rejected before RLP decoding");
        match err {
            InclusionListError::TooLarge { total: reported } => {
                assert!(reported > MAX_BYTES_PER_INCLUSION_LIST);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn at_8kib_boundary_is_accepted() {
        // Pad the canonical encoding of a single tx by repeatedly inserting
        // valid-canonical entries until just below the cap.
        let tx = sample_legacy_tx(0);
        let one = encode_canonical_bytes(&tx);
        let one_len = one.len();
        assert!(one_len > 0);

        let count = MAX_BYTES_PER_INCLUSION_LIST / one_len;
        let encoded: Vec<Bytes> = (0..count).map(|_| one.clone()).collect();

        let total: usize = encoded.iter().map(|b| b.len()).sum();
        assert!(total <= MAX_BYTES_PER_INCLUSION_LIST);

        let il = InclusionList::try_from(encoded).expect("at-or-under cap must decode");
        assert_eq!(il.transactions.len(), count);
    }

    #[test]
    fn rlp_decoding_produces_same_hash() {
        let originals = vec![
            sample_legacy_tx(0),
            sample_eip1559_tx(1),
            sample_legacy_tx(7),
        ];
        let il = InclusionList {
            transactions: originals.clone(),
        };

        let encoded: Vec<Bytes> = (&il).into();
        let decoded =
            InclusionList::try_from(encoded).expect("round-trip RLP decoding must succeed");

        assert_eq!(decoded.transactions.len(), originals.len());
        for (decoded_tx, original_tx) in decoded.transactions.iter().zip(originals.iter()) {
            assert_eq!(decoded_tx.hash(), original_tx.hash());
        }
    }

    #[test]
    fn empty_inclusion_list_roundtrips() {
        let il = InclusionList::default();
        let encoded: Vec<Bytes> = (&il).into();
        assert!(encoded.is_empty());

        let decoded = InclusionList::try_from(encoded).expect("empty list decodes");
        assert!(decoded.transactions.is_empty());
        assert_eq!(decoded.encoded_byte_len(), 0);
    }

    #[test]
    fn invalid_rlp_returns_decode_error() {
        let bogus = vec![Bytes::from(vec![0xff, 0xff, 0xff])];
        let err =
            InclusionList::try_from(bogus).expect_err("invalid canonical encoding must error");
        assert!(matches!(err, InclusionListError::Decode(_)));
    }

    #[test]
    fn ensure_rlp_encode_trait_compiles_for_transactions() {
        // Sanity-check that `Transaction` still implements `RLPEncode` so
        // future refactors don't silently drop the contract this module
        // depends on.
        let tx = sample_legacy_tx(0);
        let mut buf: Vec<u8> = Vec::new();
        <Transaction as RLPEncode>::encode(&tx, &mut buf);
        assert!(!buf.is_empty());
    }
}
