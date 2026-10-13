use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress_bounded},
};
use bytes::BufMut;
use ethrex_blockchain::mempool::Mempool;
use ethrex_common::{
    H256,
    types::{BYTES_PER_CELL, CELLS_PER_EXT_BLOB, MAX_BLOB_COUNT},
};
use ethrex_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use rustc_hash::FxHashSet;

use super::eth72::transactions::{b16_to_u128, u128_to_b16};

/// A single PeerDAS cell: 2048 bytes (BYTES_PER_CELL = 64 field elements * 32 bytes each).
pub type Cell = [u8; BYTES_PER_CELL];

/// Upper bound on transaction hashes accepted in a single GetCells/Cells message.
/// Caps allocation from a malicious peer; mirrors the GetBlockBodies-style limits.
pub const MAX_CELL_REQUEST_HASHES: usize = 256;

/// Recommended soft limit for a `GetCells` request, per devp2p `caps/eth.md`
/// ("GetCells (0x14)"). We never ask for more in one message; a peer may enforce
/// an arbitrary limit on the response, which is not a protocol violation.
pub const GET_CELLS_SOFT_LIMIT_HASHES: usize = 64;

/// Upper bound on cells per transaction in a `Cells` message: at most
/// `MAX_BLOB_COUNT` blobs * `CELLS_PER_EXT_BLOB` columns. Bounds the per-tx
/// inner-vec allocation at decode time.
pub const MAX_CELLS_PER_TX: usize = MAX_BLOB_COUNT * CELLS_PER_EXT_BLOB;

/// Upper bound on the decompressed size of a `GetCells` request: a request id, at
/// most [`MAX_CELL_REQUEST_HASHES`] 32-byte hashes (33 bytes each once RLP-tagged)
/// and a 16-byte mask, with slack for the list headers. Rejects an oversized frame
/// by its declared length, before the hash-count check can run on decoded data.
const MAX_GET_CELLS_BYTES: usize = MAX_CELL_REQUEST_HASHES * 33 + 256;

/// Recommended soft limit for a `Cells` response, per devp2p `caps/eth.md`
/// ("Cells (0x15)"). Bounds what one `GetCells` can pull out of us.
const MAX_CELLS_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// Upper bound on the decompressed size of a `Cells` response, enforced before
/// decompression so an oversized reply is rejected without materializing it.
/// Twice the soft serve limit, leaving room for a peer that overshoots by a
/// transaction, the same headroom the `PooledTransactions` path allows.
const MAX_CELLS_BYTES: usize = 2 * MAX_CELLS_RESPONSE_BYTES;

/// Cell budget for a `Cells` response we *serve*, derived from
/// [`MAX_CELLS_RESPONSE_BYTES`]. We stop before a transaction would push the
/// response past it. devp2p `caps/eth.md` lets a peer "omit entire transactions
/// from the list [...] if they are unavailable or constrained", so a partial
/// response is protocol-legal.
pub const MAX_CELLS_SERVED: usize = MAX_CELLS_RESPONSE_BYTES / BYTES_PER_CELL;

/// Worst-case cells one requested hash can pull, used to size a `GetCells`
/// request so a peer applying [`MAX_CELLS_SERVED`] can answer it in full.
pub const fn cells_per_hash(cell_mask: u128) -> usize {
    let columns = cell_mask.count_ones() as usize;
    if columns == 0 {
        1
    } else {
        columns * MAX_BLOB_COUNT
    }
}

// https://eips.ethereum.org/EIPS/eip-8070#getcells-0x14
//
// Note: the EIP schema lines show `[[hashes], cell_mask]` without a request id,
// but that is the usual eth/66+ shorthand (the schemas omit the request_id
// wrapper). EIP-8070's rationale confirms the wrapper is intended: the
// "devp2p message schema choices" section accounts for "one uint64 request_id
// and one uint8 message_type" of per-message overhead. The leading `id` field
// is therefore correct and matches the eth/66+ request/response framing.
#[derive(Debug, Clone)]
pub struct GetCells {
    pub id: u64,
    pub transaction_hashes: Vec<H256>,
    /// Bitmask of which cells are requested (128 bits, one per column index 0..127).
    pub cell_mask: u128,
}

impl GetCells {
    pub fn new(id: u64, transaction_hashes: Vec<H256>, cell_mask: u128) -> Self {
        Self {
            id,
            transaction_hashes,
            cell_mask,
        }
    }
}

impl RLPxMessage for GetCells {
    const CODE: u8 = 0x14;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        use bytes::Bytes;
        let mut encoded_data = vec![];
        let mask_bytes = Bytes::from(u128_to_b16(self.cell_mask).to_vec());
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.transaction_hashes)
            .encode_field(&mask_bytes)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        use bytes::Bytes;
        // A well-formed request is a request id, at most MAX_CELL_REQUEST_HASHES
        // hashes and a 16-byte mask; bound the declared length so an oversized
        // frame is rejected before it is materialized.
        let decompressed_data = snappy_decompress_bounded(msg_data, MAX_GET_CELLS_BYTES)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (transaction_hashes, decoder): (Vec<H256>, _) =
            decoder.decode_field("transactionHashes")?;
        let (mask_bytes, _): (Bytes, _) = decoder.decode_field("cellMask")?;

        if transaction_hashes.len() > MAX_CELL_REQUEST_HASHES {
            return Err(RLPDecodeError::Custom(
                "GetCells: too many transaction hashes".to_string(),
            ));
        }
        if mask_bytes.len() != 16 {
            return Err(RLPDecodeError::Custom(
                "GetCells cell_mask must be exactly 16 bytes".to_string(),
            ));
        }
        let mut arr = [0u8; 16];
        arr.copy_from_slice(&mask_bytes);
        let cell_mask = b16_to_u128(arr);

        Ok(Self {
            id,
            transaction_hashes,
            cell_mask,
        })
    }
}

// https://eips.ethereum.org/EIPS/eip-8070#cells-0x15
#[derive(Debug, Clone)]
pub struct Cells {
    pub id: u64,
    pub transaction_hashes: Vec<H256>,
    /// Cells for each requested transaction; inner vec length equals popcount(cell_mask).
    pub cells: Vec<Vec<Cell>>,
    /// Bitmask echoing which cells are provided.
    pub cell_mask: u128,
}

impl Cells {
    pub fn new(
        id: u64,
        transaction_hashes: Vec<H256>,
        cells: Vec<Vec<Cell>>,
        cell_mask: u128,
    ) -> Self {
        Self {
            id,
            transaction_hashes,
            cells,
            cell_mask,
        }
    }

    /// Check this response against the `GetCells` request it claims to answer.
    ///
    /// devp2p `caps/eth.md` lets a peer omit whole transactions or clear indices
    /// when it can't serve them, but never add: the `cells` bitmap "must be a
    /// subset of the corresponding request's bitmap", and a peer "sending invalid
    /// or not requested element must be disconnected". Returns `Err` with the
    /// reason so the caller can log it before dropping the peer.
    pub fn validate_requested(
        &self,
        requested_hashes: &[H256],
        requested_mask: u128,
    ) -> Result<(), CellsResponseError> {
        if self.cell_mask & !requested_mask != 0 {
            return Err(CellsResponseError::UnrequestedCellIndices);
        }
        if self
            .transaction_hashes
            .iter()
            .any(|hash| !requested_hashes.contains(hash))
        {
            return Err(CellsResponseError::UnrequestedTransaction);
        }
        Ok(())
    }
}

/// Why a `Cells` response failed validation against its request.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CellsResponseError {
    #[error("Cells response carries cell indices that were not requested")]
    UnrequestedCellIndices,
    #[error("Cells response carries a transaction hash that was not requested")]
    UnrequestedTransaction,
    #[error("Cells response does not answer any outstanding GetCells request")]
    UnknownRequestId,
}

impl RLPxMessage for Cells {
    const CODE: u8 = 0x15;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        use bytes::Bytes;
        let mut encoded_data = vec![];
        let mask_bytes = Bytes::from(u128_to_b16(self.cell_mask).to_vec());
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.transaction_hashes)
            .encode_field(&self.cells)
            .encode_field(&mask_bytes)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        use bytes::Bytes;
        // Reject an oversized response by its declared decompressed length before
        // materializing the cell matrix, as the `PooledTransactions` path does.
        // See `MAX_CELLS_BYTES`.
        let decompressed_data = snappy_decompress_bounded(msg_data, MAX_CELLS_BYTES)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (transaction_hashes, decoder): (Vec<H256>, _) =
            decoder.decode_field("transactionHashes")?;
        let (cells, decoder): (Vec<Vec<Cell>>, _) = decoder.decode_field("cells")?;
        let (mask_bytes, _): (Bytes, _) = decoder.decode_field("cellMask")?;

        if transaction_hashes.len() > MAX_CELL_REQUEST_HASHES {
            return Err(RLPDecodeError::Custom(
                "Cells: too many transaction hashes".to_string(),
            ));
        }
        // One cell vec per requested tx.
        if cells.len() != transaction_hashes.len() {
            return Err(RLPDecodeError::Custom(
                "Cells: cells count must equal transaction_hashes count".to_string(),
            ));
        }
        // Bound per-tx cell count to MAX_BLOB_COUNT blobs * CELLS_PER_EXT_BLOB columns
        // so a peer can't force a multi-hundred-MB allocation before snappy limits.
        if cells.iter().any(|v| v.len() > MAX_CELLS_PER_TX) {
            return Err(RLPDecodeError::Custom(
                "Cells: too many cells per transaction".to_string(),
            ));
        }
        if mask_bytes.len() != 16 {
            return Err(RLPDecodeError::Custom(
                "Cells cell_mask must be exactly 16 bytes".to_string(),
            ));
        }
        let mut arr = [0u8; 16];
        arr.copy_from_slice(&mask_bytes);
        let cell_mask = b16_to_u128(arr);

        Ok(Self {
            id,
            transaction_hashes,
            cells,
            cell_mask,
        })
    }
}

impl GetCells {
    /// Serve cells we hold for the requested hashes.
    ///
    /// A `Cells` message carries a single `cell_mask` covering all txs, so we
    /// serve a uniform column set: the requested columns we hold for EVERY
    /// requested tx. The response `cell_mask` is set to that served set (the EIP
    /// permits truncation), keeping it consistent with the packed cells so the
    /// receiver can reconstruct column indices unambiguously.
    ///
    /// `available_cell_mask` is used to compute the served intersection: when a
    /// tx has a full blob payload (blobs non-empty), all 128 columns are available
    /// and we compute cells on demand via `cells_for_columns`. When only sampled
    /// cells are held, the TxCells mask is used.
    ///
    /// Consequence of the uniform mask: if ANY requested hash is unknown, private
    /// or we hold no cells for it, the intersection collapses and we serve zero cells
    /// for the whole batch. This is a protocol-level limitation of the single
    /// per-message `cell_mask`, not a bug; callers should request hashes they
    /// expect us to hold together.
    ///
    /// The response is capped at [`MAX_CELLS_SERVED`] cells: an unbounded reply would
    /// let a peer pull `MAX_CELL_REQUEST_HASHES * MAX_CELLS_PER_TX` cells (~384 MiB)
    /// per request. Each distinct hash is served once, in first-seen order; the first
    /// hash that would not fit and every hash after it are dropped from the response
    /// entirely, which EIP-8070 permits ("the responder MAY truncate its `Cells`
    /// response depending on its current capacity") and which a requester sizing its
    /// batch with [`cells_per_hash`] never triggers.
    ///
    /// Expanding a blob into cells is CPU-bound KZG work, so what gets expanded is
    /// settled first by [`Self::plan`] from mempool bookkeeping alone; a request is cut
    /// to the budget before any cell is computed.
    pub fn handle(&self, mempool: &Mempool) -> Cells {
        let plan = self.plan(mempool);
        let mut served_hashes: Vec<H256> = Vec::with_capacity(plan.hashes.len());
        let mut all_cells: Vec<Vec<Cell>> = Vec::with_capacity(plan.hashes.len());
        let mut budget = MAX_CELLS_SERVED;
        for tx_hash in plan.hashes {
            let cells = get_cells_for_tx(mempool, tx_hash, plan.served_mask);
            // The plan sized this hash from its blob count; re-check the expanded
            // size so the response never exceeds the budget even if the pool
            // changed underneath us.
            if cells.len() > budget {
                break;
            }
            budget -= cells.len();
            served_hashes.push(tx_hash);
            all_cells.push(cells);
        }
        Cells::new(self.id, served_hashes, all_cells, plan.served_mask)
    }

    /// Decide which hashes and columns [`Self::handle`] will serve, without
    /// expanding any cell.
    ///
    /// Hashes are deduplicated (first occurrence wins) so a request repeating one
    /// hash costs one expansion, not one per repeat. The served column set is the
    /// intersection described on [`Self::handle`]. The cells a hash would add are
    /// `blob count * served columns`, read from the pool's bookkeeping; hashes are
    /// admitted in order until one would exceed [`MAX_CELLS_SERVED`], which drops
    /// that hash and every later one.
    pub(crate) fn plan(&self, mempool: &Mempool) -> ServePlan {
        let mut seen = FxHashSet::default();
        let distinct: Vec<H256> = self
            .transaction_hashes
            .iter()
            .copied()
            .filter(|hash| seen.insert(*hash))
            .collect();

        // Use available_cell_mask (real availability) for the intersection, so a
        // full-payload provider with u128::MAX availability actually serves all
        // requested columns.
        let mut served = self.cell_mask;
        for &tx_hash in &distinct {
            // A private tx never propagates, so its blob data is treated as
            // unavailable here, exactly like an unknown hash.
            if mempool.is_private(tx_hash).unwrap_or(true) {
                served = 0;
                break;
            }
            served &= mempool.available_cell_mask(tx_hash);
        }

        let columns = served.count_ones() as usize;
        let mut budget = MAX_CELLS_SERVED;
        let mut hashes = Vec::with_capacity(distinct.len());
        for tx_hash in distinct {
            let cells = mempool.servable_blob_count(tx_hash).saturating_mul(columns);
            if cells > budget {
                break;
            }
            budget -= cells;
            hashes.push(tx_hash);
        }
        ServePlan {
            served_mask: served,
            hashes,
        }
    }
}

/// What a `GetCells` request will be answered with, settled by [`GetCells::plan`]
/// from mempool bookkeeping alone: the distinct hashes to serve, in first-seen
/// order and cut to the [`MAX_CELLS_SERVED`] budget, and the column set served
/// for each. Building it expands no cell.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ServePlan {
    pub served_mask: u128,
    pub hashes: Vec<H256>,
}

/// Retrieve cells for `tx_hash` and the given `served_mask`.
///
/// When the tx has a full blob payload (non-empty blobs), compute cells from
/// the bundle on demand (requires `c-kzg`). Otherwise fall back to the stored
/// sampled cells.
fn get_cells_for_tx(mempool: &Mempool, tx_hash: H256, served_mask: u128) -> Vec<Cell> {
    if served_mask == 0 {
        return Vec::new();
    }
    // Try to serve from full blob payload (c-kzg only).
    #[cfg(feature = "c-kzg")]
    if let Some(bundle) = mempool.get_blobs_bundle(tx_hash).unwrap_or(None)
        && !bundle.blobs.is_empty()
        && let Ok(blob_cells) = bundle.cells_for_columns(served_mask)
    {
        // `cells_for_columns` returns one Vec per blob (each ordered by ascending
        // column). Transpose to the index-major wire order devp2p mandates for
        // `Cells`: all blobs at the lowest served column, then the next column, ...
        let col_count = served_mask.count_ones() as usize;
        let mut result = Vec::with_capacity(blob_cells.len() * col_count);
        for col_pos in 0..col_count {
            for blob_col_cells in &blob_cells {
                if let Some(cell) = blob_col_cells.get(col_pos) {
                    result.push(*cell);
                }
            }
        }
        return result;
    }
    // Fall back to stored sampled cells.
    mempool.get_tx_cells_for_mask(tx_hash, served_mask)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::{BYTES_PER_BLOB, BlobsBundle};

    /// A full-payload bundle of `blob_count` blobs. Every field element keeps a
    /// zero top byte, so each blob is a valid KZG input; commitments and proofs
    /// are not needed to serve cells.
    fn full_bundle(blob_count: usize, seed: u8) -> BlobsBundle {
        let mut blob = [0u8; BYTES_PER_BLOB];
        for (i, element) in blob.chunks_exact_mut(32).enumerate() {
            element[30] = (i % 251) as u8;
            element[31] = seed;
        }
        BlobsBundle {
            blobs: vec![blob; blob_count],
            commitments: vec![],
            proofs: vec![],
            version: 1,
        }
    }

    fn mempool_with(bundles: &[(H256, BlobsBundle)]) -> Mempool {
        let mempool = Mempool::new(100);
        for (hash, bundle) in bundles {
            mempool
                .add_blobs_bundle(*hash, bundle.clone())
                .expect("add blobs bundle");
        }
        mempool
    }

    #[test]
    fn plan_serves_each_distinct_hash_once_in_first_seen_order() {
        let a = H256::from_low_u64_be(1);
        let b = H256::from_low_u64_be(2);
        let mempool = mempool_with(&[(a, full_bundle(1, 1)), (b, full_bundle(1, 2))]);
        let mask = (1u128 << 3) | (1u128 << 70);

        let repeated = GetCells::new(1, vec![a; MAX_CELL_REQUEST_HASHES], mask);
        assert_eq!(
            repeated.plan(&mempool),
            ServePlan {
                served_mask: mask,
                hashes: vec![a]
            }
        );

        let interleaved = GetCells::new(2, vec![b, a, b, a, b], mask);
        assert_eq!(
            interleaved.plan(&mempool),
            ServePlan {
                served_mask: mask,
                hashes: vec![b, a]
            }
        );
    }

    #[test]
    fn plan_cuts_over_budget_hashes_from_blob_counts_alone() {
        // A max-blob tx at all 128 columns: one fits the budget, two do not.
        let per_tx = MAX_BLOB_COUNT * CELLS_PER_EXT_BLOB;
        assert!(per_tx <= MAX_CELLS_SERVED && 2 * per_tx > MAX_CELLS_SERVED);
        let a = H256::from_low_u64_be(1);
        let b = H256::from_low_u64_be(2);
        let mempool = mempool_with(&[
            (a, full_bundle(MAX_BLOB_COUNT, 1)),
            (b, full_bundle(MAX_BLOB_COUNT, 2)),
        ]);
        let request = GetCells::new(1, vec![a, b, a], u128::MAX);
        assert_eq!(
            request.plan(&mempool),
            ServePlan {
                served_mask: u128::MAX,
                hashes: vec![a]
            }
        );

        // A first hash that alone exceeds the budget yields an empty plan: nothing
        // is expanded for it.
        let big = H256::from_low_u64_be(3);
        let too_many_blobs = MAX_CELLS_SERVED / CELLS_PER_EXT_BLOB + 1;
        let mempool = mempool_with(&[(big, full_bundle(too_many_blobs, 3))]);
        let request = GetCells::new(2, vec![big], u128::MAX);
        assert_eq!(
            request.plan(&mempool),
            ServePlan {
                served_mask: u128::MAX,
                hashes: vec![]
            }
        );
    }

    #[test]
    fn plan_collapses_to_no_columns_on_an_unknown_hash() {
        let known = H256::from_low_u64_be(1);
        let unknown = H256::from_low_u64_be(9);
        let mempool = mempool_with(&[(known, full_bundle(1, 1))]);
        let request = GetCells::new(1, vec![known, unknown, known], u128::MAX);
        // Every distinct hash is listed, with no columns served for any of them.
        assert_eq!(
            request.plan(&mempool),
            ServePlan {
                served_mask: 0,
                hashes: vec![known, unknown]
            }
        );
    }

    #[cfg(feature = "c-kzg")]
    #[test]
    fn repeated_hash_is_served_once_with_its_cells() {
        let hash = H256::from_low_u64_be(1);
        let bundle = full_bundle(1, 7);
        let mempool = mempool_with(&[(hash, bundle.clone())]);
        let mask = (1u128 << 3) | (1u128 << 70);

        let response = GetCells::new(5, vec![hash; MAX_CELL_REQUEST_HASHES], mask).handle(&mempool);

        assert_eq!(response.id, 5);
        assert_eq!(response.cell_mask, mask);
        assert_eq!(response.transaction_hashes, vec![hash]);
        // One blob, so the index-major wire order is that blob's cells by
        // ascending column.
        let expected = bundle.cells_for_columns(mask).expect("compute cells");
        assert_eq!(response.cells, vec![expected[0].clone()]);
    }

    #[cfg(feature = "c-kzg")]
    #[test]
    fn response_follows_the_plan_within_budget() {
        let a = H256::from_low_u64_be(1);
        let b = H256::from_low_u64_be(2);
        let mempool = mempool_with(&[
            (a, full_bundle(MAX_BLOB_COUNT, 1)),
            (b, full_bundle(MAX_BLOB_COUNT, 2)),
        ]);
        let response = GetCells::new(1, vec![a, b, a], u128::MAX).handle(&mempool);
        assert_eq!(response.cell_mask, u128::MAX);
        assert_eq!(response.transaction_hashes, vec![a]);
        assert_eq!(response.cells.len(), 1);
        assert_eq!(response.cells[0].len(), MAX_BLOB_COUNT * CELLS_PER_EXT_BLOB);
    }
}
