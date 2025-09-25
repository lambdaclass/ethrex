use std::iter::repeat_n;

use crate::types::{Blob, CELLS_PER_EXT_BLOB, Commitment, Proof};

#[derive(thiserror::Error, Debug)]
pub enum KzgError {
    #[cfg(feature = "c-kzg")]
    #[error("c-kzg error: {0}")]
    CKzg(#[from] c_kzg::Error),
    #[error("kzg-rs error: {0}")]
    KzgRs(kzg_rs::KzgError),
}

impl From<kzg_rs::KzgError> for KzgError {
    fn from(value: kzg_rs::KzgError) -> Self {
        KzgError::KzgRs(value)
    }
}

// Verifies a KZG proof for blob committed data, using a Fiat-Shamir protocol
/// as defined by EIP-7594.
pub fn verify_cell_kzg_proof_batch(
    blobs: &[Blob],
    commitments: &[Commitment],
    cell_proof: &[Proof],
) -> Result<bool, KzgError> {
    #[cfg(not(feature = "c-kzg"))]
    return Ok(true);
    #[cfg(feature = "c-kzg")]
    {
        let c_kzg_settings = c_kzg::ethereum_kzg_settings(8);
        let mut cells = Vec::new();
        for blob in blobs {
            cells.extend(c_kzg_settings.compute_cells(&(*blob).into())?.into_iter());
        }
        c_kzg::KzgSettings::verify_cell_kzg_proof_batch(
            c_kzg_settings,
            &commitments
                .iter()
                .flat_map(|commitment| repeat_n((*commitment).into(), CELLS_PER_EXT_BLOB))
                .collect::<Vec<_>>(),
            &Vec::from_iter((0..blobs.len()).flat_map(|_| 0..CELLS_PER_EXT_BLOB as u64)),
            &cells,
            &cell_proof
                .iter()
                .map(|proof| (*proof).into())
                .collect::<Vec<_>>(),
        )
        .map_err(KzgError::from)
    }
}

/// Verifies a KZG proof for blob committed data, using a Fiat-Shamir protocol
/// as defined by c-kzg-4844.
pub fn verify_blob_kzg_proof(
    blob: Blob,
    commitment: Commitment,
    proof: Proof,
) -> Result<bool, KzgError> {
    #[cfg(not(feature = "c-kzg"))]
    {
        kzg_rs::KzgProof::verify_blob_kzg_proof(
            kzg_rs::Blob(blob),
            &kzg_rs::Bytes48(commitment),
            &kzg_rs::Bytes48(proof),
            &kzg_rs::get_kzg_settings(),
        )
        .map_err(KzgError::from)
    }
    #[cfg(feature = "c-kzg")]
    {
        c_kzg::KzgSettings::verify_blob_kzg_proof(
            c_kzg::ethereum_kzg_settings(8),
            &blob.into(),
            &commitment.into(),
            &proof.into(),
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
    #[cfg(not(feature = "c-kzg"))]
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
    #[cfg(feature = "c-kzg")]
    {
        c_kzg::KzgSettings::verify_kzg_proof(
            c_kzg::ethereum_kzg_settings(8),
            &commitment_bytes.into(),
            &z.into(),
            &y.into(),
            &proof_bytes.into(),
        )
        .map_err(KzgError::from)
    }
}

#[cfg(feature = "c-kzg")]
pub fn blob_to_kzg_commitment_and_proof(blob: &Blob) -> Result<(Commitment, Proof), KzgError> {
    let blob: c_kzg::Blob = (*blob).into();

    let commitment =
        c_kzg::KzgSettings::blob_to_kzg_commitment(c_kzg::ethereum_kzg_settings(8), &blob)?;
    let commitment_bytes = commitment.to_bytes();

    let proof = c_kzg::KzgSettings::compute_blob_kzg_proof(
        c_kzg::ethereum_kzg_settings(8),
        &blob,
        &commitment_bytes,
    )?;

    let proof_bytes = proof.to_bytes();

    Ok((commitment_bytes.into_inner(), proof_bytes.into_inner()))
}
