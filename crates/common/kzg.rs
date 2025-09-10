use crate::types::{Blob, Commitment, Proof};

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

#[cfg(feature = "kzg-rs")]
/// Verifies a KZG proof for blob committed data, using a Fiat-Shamir protocol
/// as defined by c-kzg-4844.
pub fn verify_blob_kzg_proof_kzg_rs(
    blob: Blob,
    commitment: Commitment,
    proof: Proof,
) -> Result<bool, KzgError> {
    kzg_rs::KzgProof::verify_blob_kzg_proof(
        kzg_rs::Blob(blob),
        &kzg_rs::Bytes48(commitment),
        &kzg_rs::Bytes48(proof),
        &kzg_rs::get_kzg_settings(),
    )
    .map_err(KzgError::from)
}

#[cfg(feature = "c-kzg")]
/// Verifies a KZG proof for blob committed data, using a Fiat-Shamir protocol
/// as defined by c-kzg-4844.
pub fn verify_blob_kzg_proof_c_kzg(
    blob: Blob,
    commitment: Commitment,
    proof: Proof,
) -> Result<bool, KzgError> {
    c_kzg::KzgProof::verify_blob_kzg_proof(
        &blob.into(),
        &commitment.into(),
        &proof.into(),
        c_kzg::ethereum_kzg_settings(),
    )
    .map_err(KzgError::from)
}

#[cfg(feature = "kzg-rs")]
/// Verifies that p(z) = y given a commitment that corresponds to the polynomial p(x) and a KZG proof
pub fn verify_kzg_proof_kzg_rs(
    commitment_bytes: [u8; 48],
    z: [u8; 32],
    y: [u8; 32],
    proof_bytes: [u8; 48],
) -> Result<bool, KzgError> {
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
/// Verifies that p(z) = y given a commitment that corresponds to the polynomial p(x) and a KZG proof
pub fn verify_kzg_proof_c_kzg(
    commitment_bytes: [u8; 48],
    z: [u8; 32],
    y: [u8; 32],
    proof_bytes: [u8; 48],
) -> Result<bool, KzgError> {
    c_kzg::KzgProof::verify_kzg_proof(
        &commitment_bytes.into(),
        &z.into(),
        &y.into(),
        &proof_bytes.into(),
        c_kzg::ethereum_kzg_settings(),
    )
    .map_err(KzgError::from)
}

#[cfg(feature = "openvm-kzg")]
/// Verifies that p(z) = y given a commitment that corresponds to the polynomial p(x) and a KZG proof
pub fn verify_kzg_proof_openvm_kzg(
    commitment_bytes: [u8; 48],
    z: [u8; 32],
    y: [u8; 32],
    proof_bytes: [u8; 48],
) -> Result<bool, KzgError> {
    openvm_kzg::KzgProof::verify_kzg_proof(
        &openvm_kzg::Bytes48::from_slice(&commitment_bytes)?,
        &openvm_kzg::Bytes32::from_slice(&z)?,
        &openvm_kzg::Bytes32::from_slice(&y)?,
        &openvm_kzg::Bytes48::from_slice(&proof_bytes)?,
        &openvm_kzg::get_kzg_settings(),
    )
    .map_err(KzgError::from)
}

#[cfg(feature = "c-kzg")]
pub fn blob_to_kzg_commitment_and_proof(blob: &Blob) -> Result<(Commitment, Proof), KzgError> {
    let blob: c_kzg::Blob = (*blob).into();

    let commitment =
        c_kzg::KzgCommitment::blob_to_kzg_commitment(&blob, c_kzg::ethereum_kzg_settings())?;
    let commitment_bytes = commitment.to_bytes();

    let proof = c_kzg::KzgProof::compute_blob_kzg_proof(
        &blob,
        &commitment_bytes,
        c_kzg::ethereum_kzg_settings(),
    )?;

    let proof_bytes = proof.to_bytes();

    Ok((commitment_bytes.into_inner(), proof_bytes.into_inner()))
}
