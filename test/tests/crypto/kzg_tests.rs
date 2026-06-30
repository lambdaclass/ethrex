//! Cell/blob KZG round-trip tests (moved from crates/common/crypto/kzg.rs).

use ethrex_crypto::kzg::{
    BYTES_PER_CELL, BYTES_PER_FIELD_ELEMENT, CELLS_PER_EXT_BLOB, FIELD_ELEMENTS_PER_BLOB,
    blob_to_kzg_commitment_and_proof, cells_to_blob, compute_cells, recover_cells_and_kzg_proofs,
};

/// Build a valid sample blob: set a small integer in the low bytes of each
/// 32-byte field element so that each element is well below the BLS modulus.
fn sample_blob() -> [u8; FIELD_ELEMENTS_PER_BLOB * BYTES_PER_FIELD_ELEMENT] {
    let mut blob = [0u8; FIELD_ELEMENTS_PER_BLOB * BYTES_PER_FIELD_ELEMENT];
    for i in 0..FIELD_ELEMENTS_PER_BLOB {
        // Bytes 28-31 of each 32-byte chunk hold a small value; bytes 0-27 stay
        // zero, guaranteeing the element is < BLS12-381 field modulus.
        let v = (i & 0xFF) as u8;
        blob[i * BYTES_PER_FIELD_ELEMENT + 28] = v;
        blob[i * BYTES_PER_FIELD_ELEMENT + 31] = (i >> 8 & 0xFF) as u8;
    }
    blob
}

/// T1: cells_to_blob reconstructs the original blob from all 128 cells, and
/// the reconstructed blob yields the same KZG commitment.
#[test]
fn cells_to_blob_roundtrips_full_column_set() {
    let blob = sample_blob();

    let cells = compute_cells(&blob).expect("compute_cells failed");
    assert_eq!(cells.len(), CELLS_PER_EXT_BLOB);

    let mut all_cells = [[0u8; BYTES_PER_CELL]; CELLS_PER_EXT_BLOB];
    for (i, c) in cells.iter().enumerate() {
        all_cells[i] = *c;
    }

    let reconstructed = cells_to_blob(&all_cells);
    assert_eq!(
        reconstructed, blob,
        "cells_to_blob must reproduce the original blob"
    );

    let (orig_commit, _) =
        blob_to_kzg_commitment_and_proof(&blob).expect("blob_to_kzg_commitment_and_proof failed");
    let (recon_commit, _) = blob_to_kzg_commitment_and_proof(&reconstructed)
        .expect("blob_to_kzg_commitment_and_proof on reconstructed failed");
    assert_eq!(
        orig_commit, recon_commit,
        "commitment must match after reconstruction"
    );
}

/// T2: recovering from exactly 64 non-contiguous columns (even indices) and
/// then calling cells_to_blob reproduces the original blob. Fewer than 64
/// columns must return an error.
#[test]
fn recover_then_reconstruct_from_64_columns() {
    let blob = sample_blob();

    let all_cells = compute_cells(&blob).expect("compute_cells failed");
    assert_eq!(all_cells.len(), CELLS_PER_EXT_BLOB);

    // 64 non-contiguous columns: even indices 0, 2, 4, ..., 126.
    let even_indices: Vec<u64> = (0..CELLS_PER_EXT_BLOB as u64).step_by(2).collect();
    assert_eq!(even_indices.len(), 64);

    let partial_cells: Vec<[u8; BYTES_PER_CELL]> = even_indices
        .iter()
        .map(|&i| all_cells[i as usize])
        .collect();

    let (recovered, _proofs) = recover_cells_and_kzg_proofs(&even_indices, &partial_cells)
        .expect("recover_cells_and_kzg_proofs failed with 64 columns");
    assert_eq!(recovered.len(), CELLS_PER_EXT_BLOB);

    let mut all_cell_arr = [[0u8; BYTES_PER_CELL]; CELLS_PER_EXT_BLOB];
    for (i, c) in recovered.iter().enumerate() {
        all_cell_arr[i] = *c;
    }

    let reconstructed = cells_to_blob(&all_cell_arr);
    assert_eq!(
        reconstructed, blob,
        "recovered blob must match the original"
    );

    // 63 columns must fail.
    let short_indices: Vec<u64> = even_indices[..63].to_vec();
    let short_cells: Vec<[u8; BYTES_PER_CELL]> = short_indices
        .iter()
        .map(|&i| all_cells[i as usize])
        .collect();
    let result = recover_cells_and_kzg_proofs(&short_indices, &short_cells);
    assert!(result.is_err(), "recovery from 63 columns must return Err");
}
