#[cfg(not(feature = "std"))]
use alloc::string::String;

// TODO: Currently, we cannot include the types crate independently of common because the crates are not yet split.
// After issue #4596 ("Split types crate from common") is resolved, update this to import the types crate directly,
// so that crypto/kzg.rs does not depend on common for type definitions.
pub const BYTES_PER_FIELD_ELEMENT: usize = 32;
pub const FIELD_ELEMENTS_PER_BLOB: usize = 4096;
pub const BYTES_PER_BLOB: usize = BYTES_PER_FIELD_ELEMENT * FIELD_ELEMENTS_PER_BLOB;
pub const FIELD_ELEMENTS_PER_EXT_BLOB: usize = 2 * FIELD_ELEMENTS_PER_BLOB;
pub const FIELD_ELEMENTS_PER_CELL: usize = 64;
pub const BYTES_PER_CELL: usize = FIELD_ELEMENTS_PER_CELL * BYTES_PER_FIELD_ELEMENT;
pub const CELLS_PER_EXT_BLOB: usize = FIELD_ELEMENTS_PER_EXT_BLOB / FIELD_ELEMENTS_PER_CELL;

// https://github.com/ethereum/c-kzg-4844?tab=readme-ov-file#precompute
// For Risc0 we need this parameter to be 0.
// For the rest we keep the value 8 due to optimizations.
#[cfg(not(feature = "risc0"))]
pub const KZG_PRECOMPUTE: u64 = 8;
#[cfg(feature = "risc0")]
pub const KZG_PRECOMPUTE: u64 = 0;

type Bytes48 = [u8; 48];
type Blob = [u8; BYTES_PER_BLOB];
type Commitment = Bytes48;
type Proof = Bytes48;

/// Schedules the Ethereum trusted setup to load on a background thread so later KZG operations avoid the first-call cost.
pub fn warm_up_trusted_setup() {
    #[cfg(feature = "c-kzg")]
    {
        let _ = std::thread::Builder::new()
            .name("kzg-warmup".into())
            .spawn(|| {
                std::hint::black_box(c_kzg::ethereum_kzg_settings(KZG_PRECOMPUTE));
            });
    }
}

#[derive(thiserror::Error, Debug)]
pub enum KzgError {
    #[cfg(feature = "c-kzg")]
    #[error("c-kzg error: {0}")]
    CKzg(#[from] c_kzg::Error),
    #[cfg(feature = "kzg-rs")]
    #[error("kzg-rs error: {0}")]
    KzgRs(kzg_rs::KzgError),
    #[cfg(feature = "openvm-kzg")]
    #[error("openvm-kzg error: {0}")]
    OpenvmKzg(openvm_kzg::KzgError),
    #[cfg(not(feature = "c-kzg"))]
    #[error("{0} is not supported without c-kzg feature enabled")]
    NotSupportedWithoutCKZG(String),
    #[error("unimplemented: {0}")]
    Unimplemented(String),
}

#[cfg(feature = "kzg-rs")]
impl From<kzg_rs::KzgError> for KzgError {
    fn from(value: kzg_rs::KzgError) -> Self {
        KzgError::KzgRs(value)
    }
}

#[cfg(feature = "openvm-kzg")]
impl From<openvm_kzg::KzgError> for KzgError {
    fn from(value: openvm_kzg::KzgError) -> Self {
        KzgError::OpenvmKzg(value)
    }
}

/// Verifies a KZG proof for blob committed data as defined by EIP-7594.
#[allow(unused_variables)]
pub fn verify_cell_kzg_proof_batch(
    blobs: &[Blob],
    commitments: &[Commitment],
    cell_proof: &[Proof],
) -> Result<bool, KzgError> {
    #[cfg(not(feature = "c-kzg"))]
    return Err(KzgError::NotSupportedWithoutCKZG(String::from(
        "Cell proof verification",
    )));
    #[cfg(feature = "c-kzg")]
    {
        let c_kzg_settings = c_kzg::ethereum_kzg_settings(KZG_PRECOMPUTE);
        let mut cells = Vec::new();
        for blob in blobs {
            let blob: c_kzg::Blob = (*blob).into();
            let cells_blob = c_kzg_settings
                .compute_cells(&blob)
                .map_err(KzgError::CKzg)?;
            cells.extend(*cells_blob);
        }
        c_kzg::KzgSettings::verify_cell_kzg_proof_batch(
            c_kzg_settings,
            &commitments
                .iter()
                .flat_map(|commitment| {
                    std::iter::repeat_n((*commitment).into(), CELLS_PER_EXT_BLOB)
                })
                .collect::<Vec<_>>(),
            &std::iter::repeat_n(0..CELLS_PER_EXT_BLOB as u64, blobs.len())
                .flatten()
                .collect::<Vec<_>>(),
            &cells,
            &cell_proof
                .iter()
                .map(|proof| (*proof).into())
                .collect::<Vec<_>>(),
        )
        .map_err(KzgError::from)
    }
}

/// Verifies a KZG proof for blob committed data, as defined by c-kzg-4844.
pub fn verify_blob_kzg_proof(
    blob: Blob,
    commitment: Commitment,
    proof: Proof,
) -> Result<bool, KzgError> {
    #[cfg(all(
        not(feature = "c-kzg"),
        not(feature = "openvm-kzg"),
        not(feature = "kzg-rs")
    ))]
    {
        let _blob = blob;
        let _commitment = commitment;
        let _proof = proof;
        return Err(KzgError::Unimplemented(
            "One of features c-kzg, openvm-kzg or kzg-rs should be active".into(),
        ));
    }
    #[cfg(all(
        not(feature = "c-kzg"),
        not(feature = "openvm-kzg"),
        feature = "kzg-rs"
    ))]
    {
        kzg_rs::KzgProof::verify_blob_kzg_proof(
            kzg_rs::Blob(blob),
            &kzg_rs::Bytes48(commitment),
            &kzg_rs::Bytes48(proof),
            &kzg_rs::get_kzg_settings(),
        )
        .map_err(KzgError::from)
    }
    #[cfg(all(not(feature = "c-kzg"), feature = "openvm-kzg"))]
    {
        Err(KzgError::Unimplemented(
            "openvm-kzg doesn't implement verify_blob_kzg_proof".into(),
        ))
    }
    #[cfg(all(feature = "c-kzg", not(feature = "openvm-kzg")))]
    {
        let c_kzg_settings = c_kzg::ethereum_kzg_settings(KZG_PRECOMPUTE);
        c_kzg_settings
            .verify_blob_kzg_proof(&blob.into(), &commitment.into(), &proof.into())
            .map_err(KzgError::from)
    }
    #[cfg(all(feature = "c-kzg", feature = "openvm-kzg"))]
    {
        compile_error!("you must enable only one of c-kzg or openvm-kzg feature flags")
    }
}

#[cfg(feature = "c-kzg")]
pub fn verify_kzg_proof_batch(
    blobs: &[Blob],
    commitments: &[Commitment],
    cell_proof: &[Proof],
) -> Result<bool, KzgError> {
    {
        // perf note: c_kzg::Blob is repr C maybe a unsafe transmute improves perf if the collect were deemed costly
        let blobs: Vec<_> = blobs.iter().map(|x| c_kzg::Blob::new(*x)).collect();
        let c_kzg_settings = c_kzg::ethereum_kzg_settings(KZG_PRECOMPUTE);
        c_kzg_settings
            .verify_blob_kzg_proof_batch(
                &blobs,
                &commitments
                    .iter()
                    .map(|x| c_kzg::Bytes48::new(*x))
                    .collect::<Vec<_>>(),
                &cell_proof
                    .iter()
                    .map(|proof| (*proof).into())
                    .collect::<Vec<_>>(),
            )
            .map_err(KzgError::from)
    }
}

/// Verifies that p(z) = y given a commitment that corresponds to the polynomial p(x) and a KZG proof
pub fn verify_kzg_proof(
    commitment_bytes: [u8; 48],
    z: [u8; 32],
    y: [u8; 32],
    proof_bytes: [u8; 48],
) -> Result<bool, KzgError> {
    #[cfg(all(
        not(feature = "c-kzg"),
        not(feature = "openvm-kzg"),
        not(feature = "kzg-rs")
    ))]
    {
        let _commitment_bytes = commitment_bytes;
        let _z = z;
        let _y = y;
        let _proof_bytes = proof_bytes;
        return Err(KzgError::Unimplemented(
            "One of features c-kzg, openvm-kzg or kzg-rs should be active".into(),
        ));
    }
    #[cfg(all(
        not(feature = "c-kzg"),
        not(feature = "openvm-kzg"),
        feature = "kzg-rs"
    ))]
    {
        kzg_rs::KzgProof::verify_kzg_proof(
            &kzg_rs::Bytes48(commitment_bytes),
            &kzg_rs::Bytes32(z),
            &kzg_rs::Bytes32(y),
            &kzg_rs::Bytes48(proof_bytes),
            &kzg_rs::get_kzg_settings(),
        )
        .map_err(KzgError::from)
    }
    #[cfg(all(not(feature = "c-kzg"), feature = "openvm-kzg"))]
    {
        openvm_kzg::KzgProof::verify_kzg_proof(
            &openvm_kzg::Bytes48::from_slice(&commitment_bytes)?,
            &openvm_kzg::Bytes32::from_slice(&z)?,
            &openvm_kzg::Bytes32::from_slice(&y)?,
            &openvm_kzg::Bytes48::from_slice(&proof_bytes)?,
            &openvm_kzg::get_kzg_settings(),
        )
        .map_err(KzgError::from)
    }
    #[cfg(all(feature = "c-kzg", not(feature = "openvm-kzg")))]
    {
        let c_kzg_settings = c_kzg::ethereum_kzg_settings(KZG_PRECOMPUTE);
        c_kzg_settings
            .verify_kzg_proof(
                &commitment_bytes.into(),
                &z.into(),
                &y.into(),
                &proof_bytes.into(),
            )
            .map_err(KzgError::from)
    }
    #[cfg(all(feature = "c-kzg", feature = "openvm-kzg"))]
    {
        compile_error!("you must enable only one of c-kzg or openvm-kzg feature flags")
    }
}

#[cfg(feature = "c-kzg")]
pub fn blob_to_kzg_commitment_and_proof(blob: &Blob) -> Result<(Commitment, Proof), KzgError> {
    let blob: c_kzg::Blob = (*blob).into();

    let c_kzg_settings = c_kzg::ethereum_kzg_settings(KZG_PRECOMPUTE);

    let commitment = c_kzg::KzgSettings::blob_to_kzg_commitment(c_kzg_settings, &blob)?;

    let commitment_bytes = commitment.to_bytes();
    let proof = c_kzg_settings.compute_blob_kzg_proof(&blob, &commitment_bytes)?;

    let proof_bytes = proof.to_bytes();

    Ok((commitment_bytes.into_inner(), proof_bytes.into_inner()))
}

/// Reconstruct a blob from its cells.
///
/// EIP-7594 lays the original blob data in the first `CELLS_PER_EXT_BLOB / 2`
/// cells (columns 0..63); the second half is the Reed-Solomon extension.
/// Concatenating the 64 data cells reproduces the blob's field elements 1:1,
/// so a full data-column set needs no KZG recovery. Verified empirically:
/// `compute_cells(blob)[0..64]` concatenated equals `blob` and yields the same
/// commitment.
///
/// `all_cells[col]` must hold the cell bytes for column `col`; only columns
/// 0..63 are read, so callers must ensure those are present.
pub fn cells_to_blob(
    all_cells: &[[u8; BYTES_PER_CELL]; CELLS_PER_EXT_BLOB],
) -> [u8; BYTES_PER_BLOB] {
    let mut blob = [0u8; BYTES_PER_BLOB];
    for col in 0..CELLS_PER_EXT_BLOB / 2 {
        let dst = &mut blob[col * BYTES_PER_CELL..(col + 1) * BYTES_PER_CELL];
        dst.copy_from_slice(&all_cells[col]);
    }
    blob
}

/// Recover all 128 cells and their KZG proofs from a partial set of cells.
/// `cell_indices` and `cells` must have equal length (≥ 64 for successful recovery).
/// Returns `(all_cells, all_proofs)` each of length 128.
#[cfg(feature = "c-kzg")]
pub fn recover_cells_and_kzg_proofs(
    cell_indices: &[u64],
    cells: &[[u8; BYTES_PER_CELL]],
) -> Result<(Vec<[u8; BYTES_PER_CELL]>, Vec<Proof>), KzgError> {
    let c_kzg_settings = c_kzg::ethereum_kzg_settings(KZG_PRECOMPUTE);
    let c_cells: Vec<c_kzg::Cell> = cells.iter().map(|c| c_kzg::Cell::new(*c)).collect();
    let (recovered_cells, recovered_proofs) =
        c_kzg_settings.recover_cells_and_kzg_proofs(cell_indices, &c_cells)?;
    let out_cells: Vec<[u8; BYTES_PER_CELL]> =
        recovered_cells.iter().map(|c| c.to_bytes()).collect();
    let out_proofs: Vec<Proof> = recovered_proofs
        .iter()
        .map(|p| p.to_bytes().into_inner())
        .collect();
    Ok((out_cells, out_proofs))
}

/// Compute all `CELLS_PER_EXT_BLOB` cells for a blob.
/// Returns one `[u8; BYTES_PER_CELL]` per cell.
#[cfg(feature = "c-kzg")]
pub fn compute_cells(blob: &Blob) -> Result<Vec<[u8; BYTES_PER_CELL]>, KzgError> {
    let c_kzg_settings = c_kzg::ethereum_kzg_settings(KZG_PRECOMPUTE);
    let c_blob: c_kzg::Blob = (*blob).into();
    let boxed = c_kzg_settings
        .compute_cells(&c_blob)
        .map_err(KzgError::CKzg)?;
    Ok(boxed.iter().map(|cell| cell.to_bytes()).collect())
}

/// Verify KZG cell proofs for a subset of cells (partial batch).
/// `commitments`, `cell_indices`, `cells`, and `proofs` must all have the same length.
#[allow(unused_variables)]
pub fn verify_cell_kzg_proof_batch_partial(
    commitments: &[Commitment],
    cell_indices: &[u64],
    cells: &[[u8; BYTES_PER_CELL]],
    proofs: &[Proof],
) -> Result<bool, KzgError> {
    #[cfg(not(feature = "c-kzg"))]
    {
        return Err(KzgError::Unimplemented(
            "verify_cell_kzg_proof_batch_partial requires the c-kzg feature".into(),
        ));
    }
    #[cfg(feature = "c-kzg")]
    {
        let c_kzg_settings = c_kzg::ethereum_kzg_settings(KZG_PRECOMPUTE);
        let c_commitments: Vec<c_kzg::Bytes48> = commitments.iter().map(|c| (*c).into()).collect();
        let c_cells: Vec<c_kzg::Cell> = cells.iter().map(|c| c_kzg::Cell::new(*c)).collect();
        let c_proofs: Vec<c_kzg::Bytes48> = proofs.iter().map(|p| (*p).into()).collect();
        c_kzg::KzgSettings::verify_cell_kzg_proof_batch(
            c_kzg_settings,
            &c_commitments,
            cell_indices,
            &c_cells,
            &c_proofs,
        )
        .map_err(KzgError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a valid sample blob: set a small integer in the low bytes of each
    /// 32-byte field element so that each element is well below the BLS modulus.
    fn sample_blob() -> [u8; BYTES_PER_BLOB] {
        let mut blob = [0u8; BYTES_PER_BLOB];
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
    #[cfg(feature = "c-kzg")]
    fn cells_to_blob_roundtrips_full_column_set() {
        let blob = sample_blob();

        let cells = compute_cells(&blob).expect("compute_cells failed");
        assert_eq!(cells.len(), CELLS_PER_EXT_BLOB);

        let mut all_cells = [[0u8; BYTES_PER_CELL]; CELLS_PER_EXT_BLOB];
        for (i, c) in cells.iter().enumerate() {
            all_cells[i] = *c;
        }

        let reconstructed = cells_to_blob(&all_cells);
        assert_eq!(reconstructed, blob, "cells_to_blob must reproduce the original blob");

        let (orig_commit, _) = blob_to_kzg_commitment_and_proof(&blob)
            .expect("blob_to_kzg_commitment_and_proof failed");
        let (recon_commit, _) = blob_to_kzg_commitment_and_proof(&reconstructed)
            .expect("blob_to_kzg_commitment_and_proof on reconstructed failed");
        assert_eq!(orig_commit, recon_commit, "commitment must match after reconstruction");
    }

    /// T2: recovering from exactly 64 non-contiguous columns (even indices) and
    /// then calling cells_to_blob reproduces the original blob. Fewer than 64
    /// columns must return an error.
    #[test]
    #[cfg(feature = "c-kzg")]
    fn recover_then_reconstruct_from_64_columns() {
        let blob = sample_blob();

        let all_cells = compute_cells(&blob).expect("compute_cells failed");
        assert_eq!(all_cells.len(), CELLS_PER_EXT_BLOB);

        // 64 non-contiguous columns: even indices 0, 2, 4, ..., 126.
        let even_indices: Vec<u64> = (0..CELLS_PER_EXT_BLOB as u64).step_by(2).collect();
        assert_eq!(even_indices.len(), 64);

        let partial_cells: Vec<[u8; BYTES_PER_CELL]> =
            even_indices.iter().map(|&i| all_cells[i as usize]).collect();

        let (recovered, _proofs) =
            recover_cells_and_kzg_proofs(&even_indices, &partial_cells)
                .expect("recover_cells_and_kzg_proofs failed with 64 columns");
        assert_eq!(recovered.len(), CELLS_PER_EXT_BLOB);

        let mut all_cell_arr = [[0u8; BYTES_PER_CELL]; CELLS_PER_EXT_BLOB];
        for (i, c) in recovered.iter().enumerate() {
            all_cell_arr[i] = *c;
        }

        let reconstructed = cells_to_blob(&all_cell_arr);
        assert_eq!(reconstructed, blob, "recovered blob must match the original");

        // 63 columns must fail.
        let short_indices: Vec<u64> = even_indices[..63].to_vec();
        let short_cells: Vec<[u8; BYTES_PER_CELL]> =
            short_indices.iter().map(|&i| all_cells[i as usize]).collect();
        let result = recover_cells_and_kzg_proofs(&short_indices, &short_cells);
        assert!(result.is_err(), "recovery from 63 columns must return Err");
    }
}

#[cfg(feature = "c-kzg")]
pub fn blob_to_commitment_and_cell_proofs(
    blob: &Blob,
) -> Result<(Commitment, Vec<Proof>), KzgError> {
    let c_kzg_settings = c_kzg::ethereum_kzg_settings(KZG_PRECOMPUTE);

    let blob: c_kzg::Blob = (*blob).into();

    let commitment = c_kzg::KzgSettings::blob_to_kzg_commitment(c_kzg_settings, &blob)?;

    let commitment_bytes = commitment.to_bytes();

    let (_cells, cell_proofs) = c_kzg_settings
        .compute_cells_and_kzg_proofs(&blob)
        .map_err(KzgError::CKzg)?;

    let cell_proofs = cell_proofs.map(|p| p.to_bytes().into_inner());

    Ok((commitment_bytes.into_inner(), cell_proofs.to_vec()))
}
