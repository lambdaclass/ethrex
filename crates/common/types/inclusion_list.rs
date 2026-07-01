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
