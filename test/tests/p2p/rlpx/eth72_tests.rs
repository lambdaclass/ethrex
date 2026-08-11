use bytes::Bytes;
use ethrex_blockchain::mempool::Mempool;
use ethrex_common::{
    Address, H256,
    types::{
        BYTES_PER_BLOB, BYTES_PER_CELL, BlobsBundle, CELLS_PER_EXT_BLOB, EIP4844Transaction,
        MAX_BLOB_COUNT, MempoolTransaction, P2PTransaction, Transaction, WrappedEIP4844Transaction,
    },
};
use ethrex_p2p::rlpx::{
    eth::{
        cells::{Cells, CellsResponseError, GetCells, MAX_CELL_REQUEST_HASHES, MAX_CELLS_SERVED},
        eth72::transactions::{
            NewPooledTransactionHashes72, PooledTransactions72, b16_to_u128, u128_to_b16,
        },
        transactions::NewPooledTransactionHashes,
    },
    message::RLPxMessage,
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn encode<M: RLPxMessage>(msg: &M) -> Vec<u8> {
    let mut buf = vec![];
    msg.encode(&mut buf).expect("encode");
    buf
}

fn npth(cell_mask: Option<u128>) -> NewPooledTransactionHashes72 {
    NewPooledTransactionHashes72::from_raw(
        Bytes::from(vec![3u8, 2u8]),
        vec![100, 200],
        vec![H256::from_low_u64_be(1), H256::from_low_u64_be(2)],
        cell_mask,
    )
}

// ── NewPooledTransactionHashes72 tests ───────────────────────────────────────

#[test]
fn cell_mask_none_encodes_to_rlp_nil() {
    // cell_mask None must encode to RLP 0x80 (nil byte string).
    let msg = npth(None);
    let encoded = encode(&msg);
    // Decoded must preserve None.
    let decoded = NewPooledTransactionHashes72::decode(&encoded).expect("decode");
    assert_eq!(decoded.cell_mask, None);
}

#[test]
fn cell_mask_some_round_trips_little_endian() {
    // Some(mask) must round-trip; wire encoding is little-endian (geth
    // CustodyBitmap layout: column i -> byte i/8, bit i%8).
    let mask = 0xA5u128 | (1u128 << 127);
    let msg = npth(Some(mask));
    let decoded = NewPooledTransactionHashes72::decode(&encode(&msg)).expect("decode");
    assert_eq!(decoded.cell_mask, Some(mask));
    assert_eq!(decoded, msg);
}

#[test]
fn eth71_decoder_rejects_the_eth72_announcement() {
    // The cell_mask field only exists from eth/72 (EIP-8070). An eth/71 peer
    // sees a four-element list and must reject it rather than ignore the extra
    // field, the way geth does ("input list has too many elements").
    for mask in [None, Some(u128::MAX)] {
        let encoded = encode(&npth(mask));
        assert!(NewPooledTransactionHashes::decode(&encoded).is_err());
    }
}

#[test]
fn u128_to_b16_is_little_endian() {
    // Internal convention: column i == bit i. Little-endian => column i lands in
    // byte i/8 at bit i%8, matching geth's types.CustodyBitmap.
    // column 0 (bit 0) -> byte 0 = 0x01
    let b = u128_to_b16(1u128);
    assert_eq!(b[0], 0x01);
    assert_eq!(b[15], 0x00);
    // column 127 (bit 127) -> byte 15 = 0x80
    let b = u128_to_b16(1u128 << 127);
    assert_eq!(b[0], 0x00);
    assert_eq!(b[15], 0x80);
    assert_eq!(b16_to_u128(b), 1u128 << 127);
}

#[test]
fn b16_to_u128_roundtrip_zero() {
    assert_eq!(b16_to_u128(u128_to_b16(0)), 0);
}

// ── GetCells / Cells tests ────────────────────────────────────────────────────

#[test]
fn get_cells_round_trip() {
    let msg = GetCells::new(
        42,
        vec![H256::from_low_u64_be(1), H256::from_low_u64_be(2)],
        0xDEAD_BEEF_u128 | (1u128 << 127),
    );
    let decoded = GetCells::decode(&encode(&msg)).expect("decode");
    assert_eq!(decoded.id, msg.id);
    assert_eq!(decoded.transaction_hashes, msg.transaction_hashes);
    assert_eq!(decoded.cell_mask, msg.cell_mask);
}

#[test]
fn cells_round_trip() {
    let cell_a: [u8; BYTES_PER_CELL] = [7u8; BYTES_PER_CELL];
    let cell_b: [u8; BYTES_PER_CELL] = [9u8; BYTES_PER_CELL];
    let msg = Cells::new(
        7,
        vec![H256::from_low_u64_be(3)],
        vec![vec![cell_a, cell_b]],
        0b1011u128,
    );
    let decoded = Cells::decode(&encode(&msg)).expect("decode");
    assert_eq!(decoded.id, msg.id);
    assert_eq!(decoded.transaction_hashes, msg.transaction_hashes);
    assert_eq!(decoded.cell_mask, msg.cell_mask);
    assert_eq!(decoded.cells, msg.cells);
}

#[test]
fn get_cells_rejects_too_many_hashes() {
    let hashes: Vec<H256> = (0..(MAX_CELL_REQUEST_HASHES as u64 + 1))
        .map(H256::from_low_u64_be)
        .collect();
    let msg = GetCells::new(1, hashes, 1);
    assert!(GetCells::decode(&encode(&msg)).is_err());
}

// ── Cells response validation against its GetCells request ───────────────────
//
// devp2p caps/eth.md: a peer may omit transactions or clear indices, but the
// response's hash list and `cells` bitmap must both be subsets of the request's.

fn cells_response(hashes: Vec<H256>, cell_mask: u128) -> Cells {
    let per_tx = vec![[0u8; BYTES_PER_CELL]; cell_mask.count_ones() as usize];
    let cells = vec![per_tx; hashes.len()];
    Cells::new(5, hashes, cells, cell_mask)
}

#[test]
fn cells_accepts_subset_of_request() {
    let requested = vec![H256::from_low_u64_be(1), H256::from_low_u64_be(2)];
    // Fewer hashes and fewer indices than requested: both legal truncations.
    let response = cells_response(vec![H256::from_low_u64_be(2)], 0b0010u128);
    assert!(response.validate_requested(&requested, 0b1011u128).is_ok());
}

#[test]
fn cells_accepts_empty_response() {
    let requested = vec![H256::from_low_u64_be(1)];
    let response = cells_response(vec![], 0);
    assert!(response.validate_requested(&requested, 0b1111u128).is_ok());
}

#[test]
fn cells_rejects_unrequested_cell_index() {
    let requested = vec![H256::from_low_u64_be(1)];
    // Bit 2 was never requested.
    let response = cells_response(requested.clone(), 0b0101u128);
    assert_eq!(
        response.validate_requested(&requested, 0b0011u128),
        Err(CellsResponseError::UnrequestedCellIndices)
    );
}

#[test]
fn cells_rejects_unrequested_transaction() {
    let requested = vec![H256::from_low_u64_be(1)];
    let response = cells_response(vec![H256::from_low_u64_be(7)], 0b0001u128);
    assert_eq!(
        response.validate_requested(&requested, 0b0011u128),
        Err(CellsResponseError::UnrequestedTransaction)
    );
}

// ── PooledTransactions72 elided round-trip ────────────────────────────────────
//
// Encodes a WrappedEIP4844Transaction via PooledTransactions72 (which elides the
// blobs list), then decodes and asserts:
//   - decoded blobs are EMPTY (elided)
//   - commitments are PRESERVED
//   - proofs are PRESERVED (cell proofs, one per blob-column pair)
#[test]
fn pooled_transactions_72_elided_blob_round_trip() {
    // Build a synthetic 1-blob bundle. KZG validity is not required here;
    // the test checks that encode_elided_canonical + PooledTransactions72
    // encode/decode preserves commitments + cell proofs while stripping blobs.
    let commitment = [0xABu8; 48];
    // cell proofs: CELLS_PER_EXT_BLOB proofs of 48 bytes each.
    let cell_proofs: Vec<[u8; 48]> = (0..CELLS_PER_EXT_BLOB).map(|i| [i as u8; 48]).collect();
    let blobs_bundle = BlobsBundle {
        blobs: vec![[0u8; BYTES_PER_BLOB]],
        commitments: vec![commitment],
        proofs: cell_proofs.clone(),
        version: 1, // Osaka cell-proof wrapper
    };

    let tx = EIP4844Transaction {
        blob_versioned_hashes: vec![ethrex_common::types::kzg_commitment_to_versioned_hash(
            &commitment,
        )],
        ..Default::default()
    };
    let wrapped = WrappedEIP4844Transaction {
        tx,
        wrapper_version: Some(1),
        blobs_bundle,
    };

    let msg = PooledTransactions72::new(
        99,
        vec![P2PTransaction::EIP4844TransactionWithBlobs(wrapped)],
    );

    let mut buf = vec![];
    msg.encode(&mut buf).expect("encode");
    let decoded = PooledTransactions72::decode(&buf).expect("decode");

    assert_eq!(decoded.id, 99);
    assert_eq!(decoded.pooled_transactions.len(), 1);

    let P2PTransaction::EIP4844TransactionWithBlobs(got) = &decoded.pooled_transactions[0] else {
        panic!("expected EIP4844TransactionWithBlobs");
    };

    // Blobs must be EMPTY (elided on the wire).
    assert!(
        got.blobs_bundle.blobs.is_empty(),
        "elided encoding must have empty blobs, got {} blobs",
        got.blobs_bundle.blobs.len()
    );

    // Commitments must be PRESERVED.
    assert_eq!(
        got.blobs_bundle.commitments,
        vec![commitment],
        "commitments must be preserved through elided round-trip"
    );

    // Cell proofs must be PRESERVED.
    assert_eq!(
        got.blobs_bundle.proofs, cell_proofs,
        "cell proofs must be preserved through elided round-trip"
    );
}

// Moved from crates/networking/p2p/rlpx/eth/eth72/transactions.rs. Exercises the
// crate-private `bytes_to_cell_mask` via the `test_utils` feature-gated shim.
#[test]
fn bytes_to_cell_mask_rejects_wrong_length() {
    use ethrex_p2p::test_utils::bytes_to_cell_mask;
    assert_eq!(bytes_to_cell_mask(&Bytes::new()).unwrap(), None);
    assert!(bytes_to_cell_mask(&Bytes::from(vec![0u8; 8])).is_err());
    assert!(bytes_to_cell_mask(&Bytes::from(vec![0u8; 16])).is_ok());
}

// ── GetCells serving ─────────────────────────────────────────────────────────

#[test]
fn get_cells_does_not_serve_a_private_transaction() {
    // --mempool.private txs never propagate, so their blob data must not leak
    // through the eth/72 cell path either.
    let mempool = Mempool::new(64);
    let tx_hash = H256::from_low_u64_be(7);
    let sender = Address::from_low_u64_be(1);
    let tx = Transaction::EIP4844Transaction(EIP4844Transaction {
        gas: 21_000,
        to: Address::from_low_u64_be(2),
        ..Default::default()
    });
    mempool
        .add_transaction_no_broadcast(
            tx_hash,
            sender,
            MempoolTransaction::new(tx, sender),
            None,
            None,
        )
        .expect("add private tx");
    mempool
        .store_cells(tx_hash, 1, vec![(0, 0, Box::new([0xAAu8; BYTES_PER_CELL]))])
        .expect("store cells");

    let response = GetCells::new(1, vec![tx_hash], 0b1).handle(&mempool);
    assert_eq!(response.cell_mask, 0, "no columns may be served");
    assert!(
        response.cells.iter().all(|c| c.is_empty()),
        "no cells may be served for a private tx"
    );
}

/// Store every column of `blob_count` blobs for `tx_hash`, so the whole 128-column
/// mask is available and each served column costs `blob_count` cells.
fn store_full_columns(mempool: &Mempool, tx_hash: H256, blob_count: usize) {
    let cells = (0..blob_count)
        .flat_map(|blob_idx| {
            (0..CELLS_PER_EXT_BLOB)
                .map(move |col| (blob_idx, col, Box::new([0xCDu8; BYTES_PER_CELL])))
        })
        .collect();
    mempool
        .store_cells(tx_hash, blob_count, cells)
        .expect("store cells");
}

#[test]
fn get_cells_truncates_at_the_serve_budget() {
    // Two max-width transactions ask for 2 * MAX_BLOB_COUNT * CELLS_PER_EXT_BLOB
    // cells, past the serve budget. EIP-8070 lets the responder truncate, so the
    // second transaction is dropped from the response rather than served.
    let mempool = Mempool::new(64);
    let first = H256::from_low_u64_be(11);
    let second = H256::from_low_u64_be(12);
    store_full_columns(&mempool, first, MAX_BLOB_COUNT);
    store_full_columns(&mempool, second, MAX_BLOB_COUNT);

    let request = GetCells::new(1, vec![first, second], u128::MAX);
    let response = request.handle(&mempool);

    assert_eq!(
        response.transaction_hashes,
        vec![first],
        "the over-budget transaction is dropped, not served empty"
    );
    let served: usize = response.cells.iter().map(Vec::len).sum();
    assert!(
        served <= MAX_CELLS_SERVED,
        "served {served} cells, budget is {MAX_CELLS_SERVED}"
    );
    response
        .validate_requested(&request.transaction_hashes, request.cell_mask)
        .expect("a truncated response is still a subset of the request");
}

#[test]
fn cells_rejects_a_response_over_the_inbound_budget() {
    // Each transaction stays within MAX_CELLS_PER_TX, but together they exceed the
    // decode budget; the declared decompressed length must be rejected before the
    // cell matrix is materialized.
    let per_tx = vec![[0u8; BYTES_PER_CELL]; MAX_BLOB_COUNT * CELLS_PER_EXT_BLOB];
    let hashes: Vec<H256> = (0..3).map(H256::from_low_u64_be).collect();
    let msg = Cells::new(1, hashes, vec![per_tx; 3], u128::MAX);
    assert!(Cells::decode(&encode(&msg)).is_err());
}

#[test]
fn an_all_zero_mask_on_a_non_blob_announcement_is_not_an_availability_claim() {
    // devp2p caps/eth.md types `cells` as B_16 and says it can be ignored when no
    // blob transactions are announced, so a conformant peer sends 16 zero bytes on
    // every non-blob announcement. Treating that as "holds no columns" would wipe
    // the peer's recorded availability and stop sampling from it.
    let non_blob = NewPooledTransactionHashes72::from_raw(
        Bytes::from(vec![2u8, 2u8]),
        vec![100, 200],
        vec![H256::from_low_u64_be(1), H256::from_low_u64_be(2)],
        Some(0),
    );
    assert!(!non_blob.announces_blob_tx());

    let with_blob = npth(Some(0xFF));
    assert!(with_blob.announces_blob_tx());
}
