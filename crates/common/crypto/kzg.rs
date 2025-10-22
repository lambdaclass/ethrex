use std::{iter::repeat_n, sync::LazyLock};

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

#[cfg(feature = "c-kzg")]
static KZG_TRUSTED_SETUP: LazyLock<&'static c_kzg::KzgSettings> =
    LazyLock::new(|| c_kzg::ethereum_kzg_settings(8));

/// Ensures the Ethereum trusted setup is loaded so later KZG operations avoid the first-call cost.
pub fn warm_up_trusted_setup() {
    #[cfg(feature = "c-kzg")]
    {
        std::hint::black_box(&KZG_TRUSTED_SETUP);
    }
}

#[derive(thiserror::Error, Debug)]
pub enum KzgError {
    #[cfg(feature = "c-kzg")]
    #[error("c-kzg error: {0}")]
    CKzg(#[from] c_kzg::Error),
    #[error("kzg-rs error: {0}")]
    KzgRs(kzg_rs::KzgError),
    #[cfg(not(feature = "c-kzg"))]
    #[error("{0} is not supported without c-kzg feature enabled")]
    NotSupportedWithoutCKZG(String),
}

impl From<kzg_rs::KzgError> for KzgError {
    fn from(value: kzg_rs::KzgError) -> Self {
        KzgError::KzgRs(value)
    }
}

/// Verifies a KZG proof for blob committed data, using a Fiat-Shamir protocol
/// as defined by EIP-7594.
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
        KZG_TRUSTED_SETUP
            .verify_blob_kzg_proof(&blob.into(), &commitment.into(), &proof.into())
            .map_err(KzgError::from)
    }
}

#[cfg(feature = "c-kzg")]
pub fn verify_kzg_proof_batch(
    blobs: &[Blob],
    commitments: &[Commitment],
    cell_proof: &[Proof],
) -> Result<bool, KzgError> {
    {
        // perf note: c_kzg::Blob is repr C maybe a unsafe transmute improves perf if needed here
        let blobs: Vec<_> = blobs.iter().map(|x| c_kzg::Blob::new(*x)).collect();
        KZG_TRUSTED_SETUP
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
        KZG_TRUSTED_SETUP
            .verify_kzg_proof(
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

    let commitment = KZG_TRUSTED_SETUP.blob_to_kzg_commitment(&blob)?;
    let commitment_bytes = commitment.to_bytes();

    let proof = KZG_TRUSTED_SETUP.compute_blob_kzg_proof(&blob, &commitment_bytes)?;

    let proof_bytes = proof.to_bytes();

    Ok((commitment_bytes.into_inner(), proof_bytes.into_inner()))
}
