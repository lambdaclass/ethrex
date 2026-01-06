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

// KZG_PRECOMPUTE constant removed - c-kzg no longer used for pevm compatibility

type Bytes48 = [u8; 48];
type Blob = [u8; BYTES_PER_BLOB];
type Commitment = Bytes48;
type Proof = Bytes48;

/// Schedules the Ethereum trusted setup to load on a background thread so later KZG operations avoid the first-call cost.
pub fn warm_up_trusted_setup() {
    // c-kzg removed for pevm compatibility - using kzg-rs instead
    // kzg-rs loads settings lazily, so no explicit warmup needed
}

#[derive(thiserror::Error, Debug)]
pub enum KzgError {
    #[error("kzg-rs error: {0}")]
    KzgRs(kzg_rs::KzgError),
    #[cfg(feature = "openvm-kzg")]
    #[error("openvm-kzg error: {0}")]
    OpenvmKzg(openvm_kzg::KzgError),
    #[error("{0} is not supported without c-kzg feature enabled")]
    NotSupportedWithoutCKZG(String),
    #[error("unimplemented: {0}")]
    Unimplemented(String),
}

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
    // c-kzg removed for pevm compatibility - cell proof verification requires c-kzg
    Err(KzgError::NotSupportedWithoutCKZG(String::from(
        "Cell proof verification",
    )))
}

/// Verifies a KZG proof for blob committed data, as defined by c-kzg-4844.
pub fn verify_blob_kzg_proof(
    blob: Blob,
    commitment: Commitment,
    proof: Proof,
) -> Result<bool, KzgError> {
    #[cfg(not(feature = "openvm-kzg"))]
    {
        kzg_rs::KzgProof::verify_blob_kzg_proof(
            kzg_rs::Blob(blob),
            &kzg_rs::Bytes48(commitment),
            &kzg_rs::Bytes48(proof),
            &kzg_rs::get_kzg_settings(),
        )
        .map_err(KzgError::from)
    }
    #[cfg(feature = "openvm-kzg")]
    {
        Err(KzgError::Unimplemented(
            "openvm-kzg doesn't implement verify_blob_kzg_proof".to_string(),
        ))
    }
}

/// Verifies KZG proofs for a batch of blobs
/// Note: c-kzg removed for pevm compatibility - this function now returns an error
#[allow(unused_variables)]
pub fn verify_kzg_proof_batch(
    blobs: &[Blob],
    commitments: &[Commitment],
    cell_proof: &[Proof],
) -> Result<bool, KzgError> {
    Err(KzgError::NotSupportedWithoutCKZG(String::from(
        "verify_kzg_proof_batch",
    )))
}

/// Verifies that p(z) = y given a commitment that corresponds to the polynomial p(x) and a KZG proof
pub fn verify_kzg_proof(
    commitment_bytes: [u8; 48],
    z: [u8; 32],
    y: [u8; 32],
    proof_bytes: [u8; 48],
) -> Result<bool, KzgError> {
    #[cfg(not(feature = "openvm-kzg"))]
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

/// Computes KZG commitment and proof for a blob
/// Note: c-kzg removed for pevm compatibility - this function now returns an error
#[allow(unused_variables)]
pub fn blob_to_kzg_commitment_and_proof(blob: &Blob) -> Result<(Commitment, Proof), KzgError> {
    Err(KzgError::NotSupportedWithoutCKZG(String::from(
        "blob_to_kzg_commitment_and_proof",
    )))
}

/// Computes KZG commitment and cell proofs for a blob
/// Note: c-kzg removed for pevm compatibility - this function now returns an error
#[allow(unused_variables)]
pub fn blob_to_commitment_and_cell_proofs(
    blob: &Blob,
) -> Result<(Commitment, Vec<Proof>), KzgError> {
    Err(KzgError::NotSupportedWithoutCKZG(String::from(
        "blob_to_commitment_and_cell_proofs",
    )))
}
