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
        return Err(KzgError::Unimplemented(
            "One of features c-kzg, openvm-kzg or kzg-rs should be active".to_string(),
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
            "openvm-kzg doesn't implement verify_blob_kzg_proof".to_string(),
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
        return Err(KzgError::Unimplemented(
            "One of features c-kzg, openvm-kzg or kzg-rs should be active".to_string(),
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
