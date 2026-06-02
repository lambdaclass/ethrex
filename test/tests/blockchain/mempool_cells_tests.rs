use ethrex_blockchain::mempool::Mempool;
use ethrex_common::{
    H256,
    types::{BYTES_PER_BLOB, BYTES_PER_CELL, BlobsBundle, CELLS_PER_EXT_BLOB},
};

// Convenience: build a fake cell of a recognizable pattern.
fn cell(byte: u8) -> Box<[u8; BYTES_PER_CELL]> {
    Box::new([byte; BYTES_PER_CELL])
}

fn h(n: u64) -> H256 {
    H256::from_low_u64_be(n)
}

// ── TxCells store/retrieve coherence ─────────────────────────────────────────

#[test]
fn store_and_get_cells_round_trip_single_blob() {
    let mp = Mempool::new(64);
    let tx = h(1);
    // Store column 0 and column 1 for blob 0.
    mp.store_cells(tx, 1, vec![(0, 0, cell(0xAA)), (0, 1, cell(0xBB))])
        .unwrap();

    // mask with bits 0 and 1 set.
    let mask = 0b11u128;
    let cells = mp.get_tx_cells_for_mask(tx, mask);
    // blob_count=1, columns 0 and 1 → 2 cells.
    assert_eq!(cells.len(), 2);
    assert!(cells[0].iter().all(|&b| b == 0xAA));
    assert!(cells[1].iter().all(|&b| b == 0xBB));
}

#[test]
fn store_and_get_cells_multi_blob_multi_column() {
    let mp = Mempool::new(64);
    let tx = h(2);
    // 2 blobs, columns 0 and 2.
    mp.store_cells(
        tx,
        2,
        vec![
            (0, 0, cell(0x11)),
            (0, 2, cell(0x22)),
            (1, 0, cell(0x33)),
            (1, 2, cell(0x44)),
        ],
    )
    .unwrap();

    let mask = 0b101u128; // columns 0 and 2
    let cells = mp.get_tx_cells_for_mask(tx, mask);
    // blob-major order: [blob0_col0, blob0_col2, blob1_col0, blob1_col2]
    assert_eq!(cells.len(), 4);
    assert!(cells[0].iter().all(|&b| b == 0x11));
    assert!(cells[1].iter().all(|&b| b == 0x22));
    assert!(cells[2].iter().all(|&b| b == 0x33));
    assert!(cells[3].iter().all(|&b| b == 0x44));
}

// ── mask() reflects a column only when present for ALL blobs ─────────────────

#[test]
fn mask_requires_cell_in_all_blobs() {
    let mp = Mempool::new(64);
    let tx = h(3);
    // 2 blobs; store column 0 for both but column 1 only for blob 0.
    mp.store_cells(
        tx,
        2,
        vec![(0, 0, cell(1)), (1, 0, cell(2)), (0, 1, cell(3))],
    )
    .unwrap();

    let got_mask = mp.get_cells_mask(tx).unwrap();
    // Column 0 is present in both blobs → bit 0 set.
    assert_eq!(got_mask & 1, 1, "column 0 must be in mask");
    // Column 1 is missing for blob 1 → bit 1 NOT set.
    assert_eq!((got_mask >> 1) & 1, 0, "column 1 must not be in mask");
}

// ── missing_custody_columns ───────────────────────────────────────────────────

#[test]
fn missing_custody_columns_reports_gaps() {
    let mp = Mempool::new(64);
    let tx = h(4);
    // Custody covers columns 0, 1, 2 (mask = 0b111).
    mp.set_custody_columns(0b111).unwrap();
    // We only have column 0 stored.
    mp.store_cells(tx, 1, vec![(0, 0, cell(0xFF))]).unwrap();

    let missing = mp.missing_custody_columns(tx).unwrap();
    // Missing = 0b111 & !0b001 = 0b110 (columns 1 and 2).
    assert_eq!(missing & 0b111, 0b110);
}

#[test]
fn missing_custody_columns_zero_when_all_held() {
    let mp = Mempool::new(64);
    let tx = h(5);
    mp.set_custody_columns(0b11).unwrap();
    mp.store_cells(tx, 1, vec![(0, 0, cell(1)), (0, 1, cell(2))])
        .unwrap();
    assert_eq!(mp.missing_custody_columns(tx).unwrap(), 0);
}

// ── prune_cells drops entries for removed txs ─────────────────────────────────

#[test]
fn prune_cells_drops_entries_not_in_pool() {
    let mp = Mempool::new(64);
    let tx = h(6);
    mp.store_cells(tx, 1, vec![(0, 0, cell(7))]).unwrap();

    // No transaction in the pool → prune_cells should remove the cell entry.
    mp.prune_cells().unwrap();
    let cells = mp.get_tx_cells_for_mask(tx, u128::MAX);
    assert!(cells.is_empty(), "cells for evicted tx must be pruned");
}

// ── provider_announcer_count + record_provider_announcement dedup ─────────────

#[test]
fn provider_announcer_count_deduplicates_same_peer() {
    let mp = Mempool::new(64);
    let tx = h(7);
    let peer_a = h(100);
    let peer_b = h(101);

    assert_eq!(mp.provider_announcer_count(tx).unwrap(), 0);

    mp.record_provider_announcement(tx, peer_a).unwrap();
    assert_eq!(mp.provider_announcer_count(tx).unwrap(), 1);

    // Same peer again — should not count twice.
    mp.record_provider_announcement(tx, peer_a).unwrap();
    assert_eq!(mp.provider_announcer_count(tx).unwrap(), 1);

    // Second peer.
    mp.record_provider_announcement(tx, peer_b).unwrap();
    assert_eq!(mp.provider_announcer_count(tx).unwrap(), 2);
}

// ── custody_generation bumps on set_custody_columns change ────────────────────

#[test]
fn custody_generation_bumps_on_change() {
    let mp = Mempool::new(64);
    let gen0 = mp.custody_generation();

    mp.set_custody_columns(0b0001).unwrap();
    let gen1 = mp.custody_generation();
    assert!(gen1 > gen0, "generation must increase on first change");

    // Identical value → no bump.
    mp.set_custody_columns(0b0001).unwrap();
    assert_eq!(mp.custody_generation(), gen1, "identical set must not bump");

    // Different value → bump.
    mp.set_custody_columns(0b0011).unwrap();
    assert!(
        mp.custody_generation() > gen1,
        "generation must bump on change"
    );
}

// ── blob_txs_missing_custody ──────────────────────────────────────────────────

#[test]
fn blob_txs_missing_custody_returns_expected_pairs() {
    let mp = Mempool::new(64);

    // Custody columns 0 and 1.
    mp.set_custody_columns(0b11).unwrap();

    // Add a blob tx to the pool (tx hash = h(10)).
    let tx_hash = h(10);
    let bundle = BlobsBundle {
        blobs: vec![[0u8; ethrex_common::types::BYTES_PER_BLOB]],
        commitments: vec![[0u8; 48]],
        proofs: vec![[0u8; 48]; CELLS_PER_EXT_BLOB],
        version: 0,
    };
    mp.add_blobs_bundle(tx_hash, bundle).unwrap();

    // Store cell for column 0 only.
    mp.store_cells(tx_hash, 1, vec![(0, 0, cell(0x01))])
        .unwrap();

    // Also add a dummy blob tx h(11) with no cells stored.
    let tx_hash2 = h(11);
    let bundle2 = BlobsBundle {
        blobs: vec![[1u8; ethrex_common::types::BYTES_PER_BLOB]],
        commitments: vec![[1u8; 48]],
        proofs: vec![[1u8; 48]; CELLS_PER_EXT_BLOB],
        version: 0,
    };
    mp.add_blobs_bundle(tx_hash2, bundle2).unwrap();

    let missing = mp.blob_txs_missing_custody().unwrap();
    // Both txs should appear; h(10) missing column 1, h(11) missing both 0 and 1.
    assert!(
        missing.len() >= 2,
        "expected at least 2 entries, got {}",
        missing.len()
    );
    // h(10): have col 0, custody = 0b11, missing = 0b10.
    let entry10 = missing.iter().find(|(h, _)| *h == tx_hash);
    assert!(entry10.is_some(), "tx_hash h(10) must appear");
    assert_eq!(entry10.unwrap().1, 0b10, "h(10) missing must be column 1");

    // h(11): have nothing, missing = 0b11.
    let entry11 = missing.iter().find(|(h, _)| *h == tx_hash2);
    assert!(entry11.is_some(), "tx_hash h(11) must appear");
    assert_eq!(
        entry11.unwrap().1,
        0b11,
        "h(11) missing must be both columns"
    );
}

// ── 2-provider gate ───────────────────────────────────────────────────────────

#[test]
fn provider_announcer_count_two_provider_gate() {
    use ethrex_blockchain::mempool::MIN_PROVIDERS_BEFORE_SAMPLING;
    let mp = Mempool::new(64);
    let tx = h(20);

    // Below gate.
    mp.record_provider_announcement(tx, h(200)).unwrap();
    assert!(
        mp.provider_announcer_count(tx).unwrap() < MIN_PROVIDERS_BEFORE_SAMPLING,
        "should be below gate after 1 announcer"
    );

    // At gate.
    mp.record_provider_announcement(tx, h(201)).unwrap();
    assert!(
        mp.provider_announcer_count(tx).unwrap() >= MIN_PROVIDERS_BEFORE_SAMPLING,
        "should reach gate after 2 announcers"
    );
}

// ── available_cell_mask (D2) ──────────────────────────────────────────────────

#[test]
fn available_cell_mask_returns_all_ones_for_full_bundle() {
    let mp = Mempool::new(64);
    let tx_hash = h(30);
    // Store a bundle with non-empty blobs.
    let bundle = BlobsBundle {
        blobs: vec![[0u8; BYTES_PER_BLOB]],
        commitments: vec![[0u8; 48]],
        proofs: vec![[0u8; 48]; CELLS_PER_EXT_BLOB],
        version: 0,
    };
    mp.add_blobs_bundle(tx_hash, bundle).unwrap();

    assert_eq!(
        mp.available_cell_mask(tx_hash),
        u128::MAX,
        "full bundle must report u128::MAX availability"
    );
}

#[test]
fn available_cell_mask_returns_sampled_mask_for_elided_bundle() {
    let mp = Mempool::new(64);
    let tx_hash = h(31);
    // Store an elided bundle (no blobs, only commitments and proofs).
    let bundle = BlobsBundle {
        blobs: vec![],
        commitments: vec![[0u8; 48]],
        proofs: vec![[0u8; 48]; CELLS_PER_EXT_BLOB],
        version: 0,
    };
    mp.add_blobs_bundle(tx_hash, bundle).unwrap();

    // Store cells for columns 0 and 1 only.
    mp.store_cells(tx_hash, 1, vec![(0, 0, cell(0xAA)), (0, 1, cell(0xBB))])
        .unwrap();

    let mask = mp.available_cell_mask(tx_hash);
    assert_eq!(mask & 0b11, 0b11, "columns 0 and 1 must be set");
    assert_eq!(mask >> 2, 0, "no other columns should be set");
}

#[test]
fn available_cell_mask_returns_zero_for_unknown_hash() {
    let mp = Mempool::new(64);
    assert_eq!(
        mp.available_cell_mask(h(99)),
        0,
        "unknown hash must report 0"
    );
}
