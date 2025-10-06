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
type Bytes48 = [u8; 48];
type Blob = [u8; BYTES_PER_BLOB];
type Commitment = Bytes48;
type Proof = Bytes48;

// Compile-time check to ensure that at least one backend feature is enabled.
#[cfg(all(
    not(feature = "c-kzg"),
    not(feature = "kzg-rs"),
    not(feature = "openvm-kzg")
))]
const _: () = {
    compile_error!(
        "One of `c-kzg`, `kzg-rs`, or `openvm-kzg` features must be enabled to use KZG functionality."
    );
};

// Compile-time check to ensure exactly one backend feature is enabled.
#[cfg(any(
    all(feature = "c-kzg", feature = "kzg-rs"),
    all(feature = "c-kzg", feature = "openvm-kzg"),
    all(feature = "kzg-rs", feature = "openvm-kzg"),
))]
const _: () = {
    compile_error!(
        "Exactly one of `c-kzg`, `kzg-rs`, or `openvm-kzg` features must be enabled to use KZG functionality."
    );
};

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
pub fn verify_cell_kzg_proof_batch(
    _blobs: &[Blob],
    _commitments: &[Commitment],
    _cell_proof: &[Proof],
) -> Result<bool, KzgError> {
    unimplemented!()
}

#[cfg(feature = "openvm-kzg")]
pub fn verify_cell_kzg_proof_batch(
    _blobs: &[Blob],
    _commitments: &[Commitment],
    _cell_proof: &[Proof],
) -> Result<bool, KzgError> {
    unimplemented!()
}

#[cfg(feature = "c-kzg")]
/// Verifies a KZG proof for blob committed data, using a Fiat-Shamir protocol
/// as defined by EIP-7594.
pub fn verify_cell_kzg_proof_batch(
    blobs: &[Blob],
    commitments: &[Commitment],
    cell_proof: &[Proof],
) -> Result<bool, KzgError> {
        use std::iter::repeat_n;
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
            &repeat_n(0..CELLS_PER_EXT_BLOB as u64, blobs.len())
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

/// Verifies a KZG proof for blob committed data, using a Fiat-Shamir protocol
/// as defined by c-kzg-4844.
///
/// Dispatches one of the enabled implementations following the hierarchy:
/// c-kzg > kzg-rs > openvm-kzg
///
/// Different implementations exist for different targets:
/// - Host (any, usually c-kzg as it's more performant)
/// - SP1 Guest (kzg-rs)
/// - Risc0 Guest (c-kzg patched)
/// - OpenVM (openvm-kzg) - TODO
///
/// There's no implementation of blob verification for openvm-kzg yet.
#[allow(clippy::needless_return)]
pub fn verify_blob_kzg_proof(
    blob: Blob,
    commitment: Commitment,
    proof: Proof,
) -> Result<bool, KzgError> {
    #[cfg(feature = "c-kzg")]
    {
        return c_kzg::KzgSettings::verify_blob_kzg_proof(
            c_kzg::ethereum_kzg_settings(8),
            &blob.into(),
            &commitment.into(),
            &proof.into(),
        )
        .map_err(KzgError::from);
    }
    #[cfg(not(feature = "c-kzg"))]
    {
        #[cfg(feature = "kzg-rs")]
        {
            return kzg_rs::KzgProof::verify_blob_kzg_proof(
                kzg_rs::Blob(blob),
                &kzg_rs::Bytes48(commitment),
                &kzg_rs::Bytes48(proof),
                &kzg_rs::get_kzg_settings(),
            )
            .map_err(KzgError::from);
        }
        #[cfg(not(feature = "kzg-rs"))]
        {
            #[cfg(feature = "openvm-kzg")]
            {
                unimplemented!(
                    "There's no implementation of blob verification for openvm-kzg yet."
                );
            }
        }
    }
}

/// Verifies that p(z) = y given a commitment that corresponds to the polynomial p(x) and a KZG proof
///
/// Dispatches one of the enabled implementations following the hierarchy:
/// c-kzg > kzg-rs > openvm-kzg
///
/// Different implementations exist for different targets:
/// - Host (any, usually c-kzg as it's more performant)
/// - SP1 Guest (kzg-rs)
/// - Risc0 Guest (c-kzg patched)
/// - OpenVM (openvm-kzg) - TODO
///
/// There's no implementation of blob verification for openvm-kzg yet.
#[allow(clippy::needless_return)]
pub fn verify_kzg_proof(
    commitment_bytes: [u8; 48],
    z: [u8; 32],
    y: [u8; 32],
    proof_bytes: [u8; 48],
) -> Result<bool, KzgError> {
    #[cfg(feature = "c-kzg")]
    {
        return c_kzg::KzgSettings::verify_kzg_proof(
            c_kzg::ethereum_kzg_settings(8),
            &commitment_bytes.into(),
            &z.into(),
            &y.into(),
            &proof_bytes.into(),
        )
        .map_err(KzgError::from);
    }
    #[cfg(not(feature = "c-kzg"))]
    {
        #[cfg(feature = "kzg-rs")]
        {
            return kzg_rs::KzgProof::verify_kzg_proof(
                &kzg_rs::Bytes48(commitment_bytes),
                &kzg_rs::Bytes32(z),
                &kzg_rs::Bytes32(y),
                &kzg_rs::Bytes48(proof_bytes),
                &kzg_rs::get_kzg_settings(),
            )
            .map_err(KzgError::from);
        }
        #[cfg(not(feature = "kzg-rs"))]
        {
            #[cfg(feature = "openvm-kzg")]
            {
                return openvm_kzg::KzgProof::verify_kzg_proof(
                    &openvm_kzg::Bytes48::from_slice(&commitment_bytes)?,
                    &openvm_kzg::Bytes32::from_slice(&z)?,
                    &openvm_kzg::Bytes32::from_slice(&y)?,
                    &openvm_kzg::Bytes48::from_slice(&proof_bytes)?,
                    &openvm_kzg::get_kzg_settings(),
                )
                .map_err(KzgError::from);
            }
        }
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
