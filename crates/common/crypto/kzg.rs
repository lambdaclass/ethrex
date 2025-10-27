use std::{
    cell::LazyCell,
    iter::repeat_n,
    sync::{LazyLock, OnceLock},
};

use kzg::{
    eip_4844::load_trusted_setup_filename_rust,
    eip_7594::ZBackend,
    kzg_proofs::{KZGSettings, generate_trusted_setup},
};
use kzg_traits::{
    DAS, EcBackend, Fr, G1,
    eip_4844::{bytes_to_blob, load_trusted_setup_rust, load_trusted_setup_string},
    eth::{
        c_bindings::{compute_cells_and_kzg_proofs, verify_cell_kzg_proof_batch},
        eip_7594::compute_cells_raw,
    },
};

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

/// Ensures the Ethereum trusted setup is loaded so later KZG operations avoid the first-call cost.
pub fn warm_up_trusted_setup() {
    #[cfg(feature = "c-kzg")]
    {
        let _ = c_kzg::ethereum_kzg_settings(8);
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

static TRUSTED_SETUP_STRING: &str = include_str!("./kzg_trusted_setup.txt");
static TRUSTED_SETUP: LazyLock<KZGSettings> = LazyLock::new(|| {
    dbg!("loading trusted setup string");
    let (g1_monomial_bytes, g1_lagrange_bytes, g2_monomial_bytes) =
        load_trusted_setup_string(TRUSTED_SETUP_STRING).unwrap();
    dbg!("loading trusted setup rust");
    let t = load_trusted_setup_rust(&g1_monomial_bytes, &g1_lagrange_bytes, &g2_monomial_bytes).unwrap();
    dbg!("loaded trusted setup");
    t
});

/// Verifies a KZG proof for blob committed data, using a Fiat-Shamir protocol
/// as defined by EIP-7594.
/// TODO: change doc
pub fn verify_cell_kzg_proof_batch_our(
    blobs: &[Blob],
    commitments: &[Commitment],
    cell_proofs: &[Proof],
) -> Result<bool, KzgError> {
    #[cfg(not(feature = "c-kzg"))]
    {
        dbg!("starting cell verification");
        type ZG1 = <ZBackend as EcBackend>::G1;

        let mut cells = Vec::with_capacity(blobs.len());
        for bytes in blobs {
            dbg!("convert bytes to blob");
            let blob = bytes_to_blob(bytes).unwrap();
            dbg!("init blob_cells vec");
            let mut blob_cells = Vec::with_capacity(CELLS_PER_EXT_BLOB);
            dbg!("compute cells");
            <KZGSettings as DAS<ZBackend>>::compute_cells_and_kzg_proofs(
                &TRUSTED_SETUP,
                Some(&mut blob_cells),
                None,
                &blob,
            )
            .unwrap();
            dbg!("cells extend");
            cells.extend(blob_cells.into_iter());
        }

        dbg!("convert commitments");
        let commitments: Vec<_> = commitments
            .iter()
            .map(|commitment| ZG1::from_bytes(commitment).unwrap())
            .collect();
        dbg!("convert cell_proofs");
        let cell_proofs: Vec<_> = cell_proofs
            .iter()
            .map(|proof| ZG1::from_bytes(proof).unwrap())
            .collect();
        dbg!("convert cell_dincies");
        let cell_indices: Vec<_> = repeat_n(0..CELLS_PER_EXT_BLOB, blobs.len())
            .flatten()
            .collect();
        dbg!("verify_cell_kzg_proof_batch");
        return Ok(<KZGSettings as DAS<ZBackend>>::verify_cell_kzg_proof_batch(
            &TRUSTED_SETUP,
            &commitments,
            &cell_indices,
            &cells,
            &cell_proofs,
        )
        .unwrap());
    }
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
                .flat_map(|commitment| repeat_n((*commitment).into(), CELLS_PER_EXT_BLOB))
                .collect::<Vec<_>>(),
            &repeat_n(0..CELLS_PER_EXT_BLOB as u64, blobs.len())
                .flatten()
                .collect::<Vec<_>>(),
            &cells,
            &cell_proofs
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
            c_kzg::ethereum_kzg_settings(KZG_PRECOMPUTE),
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
            c_kzg::ethereum_kzg_settings(KZG_PRECOMPUTE),
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

    let commitment = c_kzg::KzgSettings::blob_to_kzg_commitment(
        c_kzg::ethereum_kzg_settings(KZG_PRECOMPUTE),
        &blob,
    )?;
    let commitment_bytes = commitment.to_bytes();

    let proof = c_kzg::KzgSettings::compute_blob_kzg_proof(
        c_kzg::ethereum_kzg_settings(KZG_PRECOMPUTE),
        &blob,
        &commitment_bytes,
    )?;

    let proof_bytes = proof.to_bytes();

    Ok((commitment_bytes.into_inner(), proof_bytes.into_inner()))
}

#[cfg(feature = "c-kzg")]
pub fn blob_to_commitment_and_cell_proofs(
    blob: &Blob,
) -> Result<(Commitment, Vec<Proof>), KzgError> {
    let c_kzg_settings = c_kzg::ethereum_kzg_settings(8);
    let blob: c_kzg::Blob = (*blob).into();
    let commitment =
        c_kzg::KzgSettings::blob_to_kzg_commitment(c_kzg::ethereum_kzg_settings(8), &blob)?;
    let commitment_bytes = commitment.to_bytes();

    let (_cells, cell_proofs) = c_kzg_settings
        .compute_cells_and_kzg_proofs(&blob)
        .map_err(KzgError::CKzg)?;
    let cell_proofs = cell_proofs.map(|p| p.to_bytes().into_inner());
    Ok((commitment_bytes.into_inner(), cell_proofs.to_vec()))
}
