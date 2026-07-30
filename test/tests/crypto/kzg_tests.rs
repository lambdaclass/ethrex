//! Cell/blob KZG round-trip tests (moved from crates/common/crypto/kzg.rs).

use ethrex_crypto::kzg::{
    BYTES_PER_CELL, BYTES_PER_FIELD_ELEMENT, CELLS_PER_EXT_BLOB, FIELD_ELEMENTS_PER_BLOB,
    blob_to_commitment_and_cell_proofs, blob_to_kzg_commitment_and_proof, cells_to_blob,
    compute_cells, recover_cells_and_kzg_proofs, verify_cell_kzg_proof_batch_partial,
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

/// T5: cell-proof batch verification accepts a valid (commitment, cells, proofs)
/// triple for the full 128-column set. This is the path EIP-8070 uses to verify
/// sampled cells against their proofs before storing them.
#[test]
fn verify_cell_proof_batch_accepts_valid_cells() {
    let blob = sample_blob();
    let (commitment, proofs) =
        blob_to_commitment_and_cell_proofs(&blob).expect("commitment_and_cell_proofs");
    assert_eq!(proofs.len(), CELLS_PER_EXT_BLOB);

    let cells = compute_cells(&blob).expect("compute_cells");
    let cell_indices: Vec<u64> = (0..CELLS_PER_EXT_BLOB as u64).collect();
    let commitments = vec![commitment; CELLS_PER_EXT_BLOB];

    let ok = verify_cell_kzg_proof_batch_partial(&commitments, &cell_indices, &cells, &proofs)
        .expect("verification must not error on valid input");
    assert!(ok, "valid cells+proofs must verify as true");
}

/// T6: a single corrupted cell must fail batch verification (must not verify as
/// true). Guards the "verify before store" invariant against tampered cell data.
#[test]
fn verify_cell_proof_batch_rejects_corrupted_cell() {
    let blob = sample_blob();
    let (commitment, proofs) =
        blob_to_commitment_and_cell_proofs(&blob).expect("commitment_and_cell_proofs");

    let mut cells = compute_cells(&blob).expect("compute_cells");
    // Flip a byte in one cell so it no longer matches its proof.
    cells[5][0] ^= 0xFF;

    let cell_indices: Vec<u64> = (0..CELLS_PER_EXT_BLOB as u64).collect();
    let commitments = vec![commitment; CELLS_PER_EXT_BLOB];

    let result = verify_cell_kzg_proof_batch_partial(&commitments, &cell_indices, &cells, &proofs);
    assert!(
        !matches!(result, Ok(true)),
        "a corrupted cell must not verify as true, got {result:?}"
    );
}

/// T7: a single corrupted proof must fail batch verification (must not verify as
/// true), whether the tampered bytes decode to a wrong point (Ok(false)) or an
/// invalid encoding (Err).
#[test]
fn verify_cell_proof_batch_rejects_corrupted_proof() {
    let blob = sample_blob();
    let (commitment, mut proofs) =
        blob_to_commitment_and_cell_proofs(&blob).expect("commitment_and_cell_proofs");

    let cells = compute_cells(&blob).expect("compute_cells");
    // Corrupt one proof.
    proofs[7][0] ^= 0xFF;

    let cell_indices: Vec<u64> = (0..CELLS_PER_EXT_BLOB as u64).collect();
    let commitments = vec![commitment; CELLS_PER_EXT_BLOB];

    let result = verify_cell_kzg_proof_batch_partial(&commitments, &cell_indices, &cells, &proofs);
    assert!(
        !matches!(result, Ok(true)),
        "a corrupted proof must not verify as true, got {result:?}"
    );
}
