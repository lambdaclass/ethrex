// TODO: Currently, we cannot include the types crate independently of common because the crates are not yet split.
// After issue #4596 ("Split types crate from common") is resolved, update this to import the types crate directly,
// so that crypto/kzg.rs does not depend on common for type definitions.
pub const BYTES_PER_FIELD_ELEMENT: usize = 32;
pub const FIELD_ELEMENTS_PER_BLOB: usize = 4096;
pub const BYTES_PER_BLOB: usize = BYTES_PER_FIELD_ELEMENT * FIELD_ELEMENTS_PER_BLOB;
type Bytes48 = [u8; 48];
type Blob = [u8; BYTES_PER_BLOB];
type Commitment = Bytes48;
type Proof = Bytes48;

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
    #[cfg(all(
        not(feature = "c-kzg"),
        not(feature = "kzg-rs"),
        not(feature = "openvm-kzg")
    ))]
    #[error(
        "no kzg backend enabled, enable at least one of `c-kzg`, `kzg-rs` or `openvm-kzg` features."
    )]
    NoBackendEnabled,
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

/// Verifies a KZG proof for blob committed data, using a Fiat-Shamir protocol
/// as defined by c-kzg-4844.
///
/// Dispatches one of the enabled implementations following the hierarchy:
/// c-kzg > kzg-rs
///
/// Different implementations exist for different targets:
/// - Host (any, usually c-kzg as it's more performant)
/// - SP1 Guest (kzg-rs)
/// - Risc0 Guest (c-kzg patched)
///
/// There's no implementation of blob verification for openvm-kzg yet.
pub fn verify_blob_kzg_proof(
    blob: Blob,
    commitment: Commitment,
    proof: Proof,
) -> Result<bool, KzgError> {
    #[cfg(all(
        not(feature = "c-kzg"),
        not(feature = "kzg-rs"),
        not(feature = "openvm-kzg")
    ))]
    {
        compile_error!(
            "Either the `c-kzg`, `kzg-rs` or `openvm-kzg` feature must be enabled to use KZG functionality."
        );
        return Ok(false);
    }
    #[cfg(feature = "kzg-rs")]
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
        c_kzg::KzgProof::verify_blob_kzg_proof(
            &blob.into(),
            &commitment.into(),
            &proof.into(),
            c_kzg::ethereum_kzg_settings(),
        )
        .map_err(KzgError::from)
    }
    #[cfg(feature = "openvm-kzg")]
    {
        unimplemented!("There's no implementation of blob verification for openvm-kzg yet.");
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
        not(feature = "kzg-rs"),
        not(feature = "openvm-kzg")
    ))]
    {
        compile_error!(
            "Either the `c-kzg`, `kzg-rs` or `openvm-kzg` feature must be enabled to use KZG functionality."
        );
        return Ok(false);
    }
    #[cfg(feature = "kzg-rs")]
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
