use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_blockchain::mempool::Mempool;
use ethrex_common::{H256, types::BYTES_PER_CELL};
use ethrex_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};

use super::eth72::transactions::{b16_to_u128, u128_to_b16};

/// A single PeerDAS cell: 2048 bytes (BYTES_PER_CELL = 64 field elements * 32 bytes each).
pub type Cell = [u8; BYTES_PER_CELL];

/// Upper bound on transaction hashes accepted in a single GetCells/Cells message.
/// Caps allocation from a malicious peer; mirrors the GetBlockBodies-style limits.
pub const MAX_CELL_REQUEST_HASHES: usize = 256;

/// Upper bound on cells per transaction in a `Cells` message: at most
/// MAX_BLOBS_PER_TX (6) blobs * CELLS_PER_EXT_BLOB (128) columns. Bounds the
/// per-tx inner-vec allocation at decode time.
pub const MAX_CELLS_PER_TX: usize = 6 * 128;

// https://eips.ethereum.org/EIPS/eip-8070#getcells-0x14
//
// Note: the EIP schema text shows `[[hashes], cell_mask]` without a request id,
// but eth/66+ wraps every request/response pair with a request id, and the
// EIP-8070 rationale explicitly relies on "request_id correlation" for
// concurrent/unordered responses. The leading `id` field is therefore correct;
// confirm the exact framing against geth/reth before cross-client devnet.
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
        let decompressed_data = snappy_decompress(msg_data)?;
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
        let decompressed_data = snappy_decompress(msg_data)?;
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
        // Bound per-tx cell count to MAX_BLOBS_PER_TX (6) * CELLS_PER_EXT_BLOB (128)
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
    /// Consequence of the uniform mask: if ANY requested hash is unknown (or we
    /// hold no cells for it), the intersection collapses and we serve zero cells
    /// for the whole batch. This is a protocol-level limitation of the single
    /// per-message `cell_mask`, not a bug; callers should request hashes they
    /// expect us to hold together. See OQ-2 in the plan.
    pub fn handle(&self, mempool: &Mempool) -> Cells {
        // D2c: use available_cell_mask (real availability) for the intersection,
        // so a full-payload provider with u128::MAX availability actually serves
        // all requested columns.
        let mut served = self.cell_mask;
        for &tx_hash in &self.transaction_hashes {
            served &= mempool.available_cell_mask(tx_hash);
        }
        let mut all_cells: Vec<Vec<Cell>> = Vec::with_capacity(self.transaction_hashes.len());
        for &tx_hash in &self.transaction_hashes {
            let cells = get_cells_for_tx(mempool, tx_hash, served);
            all_cells.push(cells);
        }
        Cells::new(self.id, self.transaction_hashes.clone(), all_cells, served)
    }
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
        // cells_for_columns returns one Vec per blob; flatten blob-major.
        let col_count = served_mask.count_ones() as usize;
        let mut result = Vec::with_capacity(blob_cells.len() * col_count);
        for blob_col_cells in &blob_cells {
            for cell in blob_col_cells {
                result.push(*cell);
            }
        }
        return result;
    }
    // Fall back to stored sampled cells.
    mempool.get_tx_cells_for_mask(tx_hash, served_mask)
}
