use bytes::Bytes;
use ethrex_common::{
    H256,
    types::{
        BYTES_PER_BLOB, BYTES_PER_CELL, BlobsBundle, CELLS_PER_EXT_BLOB, EIP4844Transaction,
        P2PTransaction, WrappedEIP4844Transaction,
    },
};
use ethrex_p2p::rlpx::{
    eth::{
        cells::{Cells, GetCells, MAX_CELL_REQUEST_HASHES},
        eth72::transactions::{
            NewPooledTransactionHashes72, PooledTransactions72, b16_to_u128, u128_to_b16,
        },
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
fn cell_mask_some_round_trips_big_endian() {
    // Some(mask) must round-trip and encode big-endian (high bit in first byte).
    let mask = 0xA5u128 | (1u128 << 127);
    let msg = npth(Some(mask));
    let decoded = NewPooledTransactionHashes72::decode(&encode(&msg)).expect("decode");
    assert_eq!(decoded.cell_mask, Some(mask));
    assert_eq!(decoded, msg);
}

#[test]
fn u128_to_b16_is_big_endian() {
    // bit 127 → byte 0 = 0x80
    let b = u128_to_b16(1u128 << 127);
    assert_eq!(b[0], 0x80);
    assert_eq!(b[15], 0x00);
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
