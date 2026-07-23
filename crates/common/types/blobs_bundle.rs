use crate::serde_utils;
#[cfg(feature = "c-kzg")]
use crate::types::Fork;
use crate::types::constants::VERSIONED_HASH_VERSION_KZG;
use crate::{Bytes, H256};

use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use serde::{Deserialize, Serialize};

use super::{BYTES_PER_BLOB, CELLS_PER_EXT_BLOB, SAFE_BYTES_PER_BLOB};

pub type Bytes48 = [u8; 48];
pub type Blob = [u8; BYTES_PER_BLOB];
pub type Commitment = Bytes48;
pub type Proof = Bytes48;
pub type BlobTuple = (Box<Blob>, Commitment, Vec<Proof>);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Struct containing all the blobs for a blob transaction, along with the corresponding commitments and proofs
pub struct BlobsBundle {
    #[serde(with = "serde_utils::blob::vec")]
    pub blobs: Vec<Blob>,
    #[serde(with = "serde_utils::bytes48::vec")]
    pub commitments: Vec<Commitment>,
    #[serde(with = "serde_utils::bytes48::vec")]
    pub proofs: Vec<Proof>,
    /// Sidecar wrapper version. Empty accumulators default to v1 so payload
    /// builders and validium paths stay consistent with the v1-only policy.
    #[serde(skip, default = "default_blob_sidecar_version")]
    pub version: u8,
}

fn default_blob_sidecar_version() -> u8 {
    1
}

impl Default for BlobsBundle {
    fn default() -> Self {
        Self {
            blobs: Vec::new(),
            commitments: Vec::new(),
            proofs: Vec::new(),
            version: default_blob_sidecar_version(),
        }
    }
}

pub fn blob_from_bytes(bytes: Bytes) -> Result<Blob, BlobsBundleError> {
    // This functions moved from `l2/utils/eth_client/transaction.rs`
    // We set the first byte of every 32-bytes chunk to 0x00
    // so it's always under the field module.
    if bytes.len() > SAFE_BYTES_PER_BLOB {
        return Err(BlobsBundleError::BlobDataInvalidBytesLength);
    }

    let mut buf = [0u8; BYTES_PER_BLOB];
    buf[..(bytes.len() * 32).div_ceil(31)].copy_from_slice(
        &bytes
            .chunks(31)
            .map(|x| [&[0x00], x].concat())
            .collect::<Vec<_>>()
            .concat(),
    );

    Ok(buf)
}

pub fn bytes_from_blob(blob: Bytes) -> [u8; SAFE_BYTES_PER_BLOB] {
    let mut buf = [0u8; SAFE_BYTES_PER_BLOB];
    buf.copy_from_slice(
        &blob
            .chunks(32)
            .map(|x| x[1..].to_vec())
            .collect::<Vec<_>>()
            .concat(),
    );

    buf
}

pub fn kzg_commitment_to_versioned_hash(data: &Commitment) -> H256 {
    use sha2::{Digest, Sha256};
    let mut versioned_hash: [u8; 32] = Sha256::digest(data).into();
    versioned_hash[0] = VERSIONED_HASH_VERSION_KZG;
    versioned_hash.into()
}

/// Compute a single EIP-4844 KZG polynomial proof for `blob`.
///
/// This is intentionally separate from `create_from_blobs` (which always produces
/// v1 cell-proof sidecars). Use this only when a single KZG proof is needed for a
/// non-P2P purpose, such as supplying `ProverInputData.blob_proof` to the ZK prover.
#[cfg(feature = "c-kzg")]
pub fn blob_to_kzg_proof(blob: &Blob) -> Result<Proof, BlobsBundleError> {
    use ethrex_crypto::kzg::blob_to_kzg_commitment_and_proof;
    let (_, proof) = blob_to_kzg_commitment_and_proof(blob)?;
    Ok(proof)
}

impl BlobsBundle {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty() && self.commitments.is_empty() && self.proofs.is_empty()
    }

    // In the future we might want to provide a new method that calculates the commitments and proofs using the following.
    #[cfg(feature = "c-kzg")]
    pub fn create_from_blobs(blobs: &Vec<Blob>) -> Result<Self, BlobsBundleError> {
        use ethrex_crypto::kzg::blob_to_commitment_and_cell_proofs;
        let mut commitments = Vec::new();
        let mut proofs = Vec::new();

        // Always produce v1 (EIP-7594) cell-proof sidecars.
        for blob in blobs {
            let (commitment, cell_proofs) = blob_to_commitment_and_cell_proofs(blob)?;
            commitments.push(commitment);
            proofs.extend(cell_proofs);
        }

        Ok(Self {
            blobs: blobs.clone(),
            commitments,
            proofs,
            version: 1,
        })
    }

    pub fn generate_versioned_hashes(&self) -> Vec<H256> {
        self.commitments
            .iter()
            .map(kzg_commitment_to_versioned_hash)
            .collect()
    }

    /// Given an index returns all or nothing `BlobTuple` if either of the commitment, proof or
    /// blob is not found then it will return None instead of Partial data.
    pub fn get_blob_tuple_by_index(&self, index: usize) -> Option<BlobTuple> {
        let blob = Box::new(*self.blobs.get(index)?);
        let commitment = *self.commitments.get(index)?;
        let proofs = self.proofs.chunks(CELLS_PER_EXT_BLOB).nth(index)?.to_vec();
        Some((blob, commitment, proofs))
    }

    /// Validates the canonical v1 sidecar layout without performing KZG verification.
    ///
    /// Empty bundles are structurally valid here because L2 validium batches and
    /// payload accumulators may legitimately contain no sidecar. Callers that
    /// require at least one blob must enforce that separately.
    pub fn validate_v1_structure(&self) -> Result<(), BlobsBundleError> {
        if self.version != 1 {
            return Err(BlobsBundleError::InvalidBlobVersionForFork);
        }
        if self.blobs.len() != self.commitments.len()
            || self.blobs.len() * CELLS_PER_EXT_BLOB != self.proofs.len()
        {
            return Err(BlobsBundleError::BlobsBundleWrongLen);
        }
        Ok(())
    }

    /// Appends a canonical v1 sidecar to this bundle.
    ///
    /// A completely empty bundle is treated as an accumulator regardless of its
    /// default version. Non-empty bundles must have the v1 cell-proof layout.
    pub fn append_v1(&mut self, other: Self) -> Result<(), BlobsBundleError> {
        if other.is_empty() {
            return Ok(());
        }
        other.validate_v1_structure()?;

        if self.is_empty() {
            *self = other;
            return Ok(());
        }
        self.validate_v1_structure()?;

        self.blobs.extend(other.blobs);
        self.commitments.extend(other.commitments);
        self.proofs.extend(other.proofs);
        self.version = 1;
        Ok(())
    }

    /// Full blob bundle validation: structural checks + KZG cryptographic proof verification.
    #[cfg(feature = "c-kzg")]
    pub fn validate(
        &self,
        tx: &super::EIP4844Transaction,
        fork: super::Fork,
    ) -> Result<(), BlobsBundleError> {
        self.validate_cheap(tx, fork)?;
        self.verify_kzg_proofs()
    }

    /// Verifies EIP-7594 cell KZG proofs against the blobs and commitments.
    #[cfg(feature = "c-kzg")]
    pub fn verify_kzg_proofs(&self) -> Result<(), BlobsBundleError> {
        self.validate_v1_structure()?;
        let valid = ethrex_crypto::kzg::verify_cell_kzg_proof_batch(
            &self.blobs,
            &self.commitments,
            &self.proofs,
        )?;
        if !valid {
            return Err(BlobsBundleError::BlobToCommitmentAndProofError);
        }
        Ok(())
    }

    /// Validates blob bundle structure without expensive KZG cryptographic verification.
    /// Used in P2P validation where full KZG is deferred to mempool insertion
    /// (after dedup check), avoiding redundant proof verification for the same
    /// blob tx received from multiple peers.
    #[cfg(feature = "c-kzg")]
    pub fn validate_cheap(
        &self,
        tx: &super::EIP4844Transaction,
        fork: super::Fork,
    ) -> Result<(), BlobsBundleError> {
        let max_blobs = max_blobs_per_block(fork);
        let blob_count = self.blobs.len();

        if blob_count > max_blobs {
            return Err(BlobsBundleError::MaxBlobsExceeded);
        }

        // EIP-7594: a single transaction may carry at most MAX_BLOB_COUNT (6) blobs,
        // independent of the higher per-block limit.
        if fork >= Fork::Osaka && blob_count > MAX_BLOB_COUNT {
            return Err(BlobsBundleError::MaxBlobsExceeded);
        }

        if blob_count == 0 {
            return Err(BlobsBundleError::BlobBundleEmptyError);
        }

        // Only v1 (EIP-7594 cell-proof) sidecars are accepted. v0 (EIP-4844 blob-proof)
        // sidecars are no longer valid at any fork, following go-ethereum#35191.
        self.validate_v1_structure()?;

        if blob_count != tx.blob_versioned_hashes.len() {
            return Err(BlobsBundleError::BlobsBundleWrongLen);
        };

        self.validate_blob_commitment_hashes(&tx.blob_versioned_hashes)?;

        Ok(())
    }

    pub fn validate_blob_commitment_hashes(
        &self,
        blob_versioned_hashes: &[H256],
    ) -> Result<(), BlobsBundleError> {
        if self.commitments.len() != blob_versioned_hashes.len() {
            return Err(BlobsBundleError::BlobVersionedHashesError);
        }
        for (commitment, blob_versioned_hash) in
            self.commitments.iter().zip(blob_versioned_hashes.iter())
        {
            if *blob_versioned_hash != kzg_commitment_to_versioned_hash(commitment) {
                return Err(BlobsBundleError::BlobVersionedHashesError);
            }
        }
        Ok(())
    }
}

impl RLPEncode for BlobsBundle {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let encoder = Encoder::new(buf);
        encoder
            .encode_field(&self.blobs)
            .encode_field(&self.commitments)
            .encode_field(&self.proofs)
            .encode_optional_field(&(self.version != 0).then_some(self.version))
            .finish();
    }
}

impl RLPDecode for BlobsBundle {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (blobs, decoder) = decoder.decode_field("blobs")?;
        let (commitments, decoder) = decoder.decode_field("commitments")?;
        let (proofs, decoder) = decoder.decode_field("proofs")?;
        let (version, decoder) = decoder.decode_optional_field();
        Ok((
            Self {
                blobs,
                commitments,
                proofs,
                version: version.unwrap_or_else(default_blob_sidecar_version),
            },
            decoder.finish()?,
        ))
    }
}

#[cfg(feature = "c-kzg")]
const MAX_BLOB_COUNT: usize = 6;
#[cfg(feature = "c-kzg")]
const MAX_BLOB_COUNT_ELECTRA: usize = 9;

#[cfg(feature = "c-kzg")]
fn max_blobs_per_block(fork: crate::types::Fork) -> usize {
    if fork >= crate::types::Fork::Prague {
        MAX_BLOB_COUNT_ELECTRA
    } else {
        MAX_BLOB_COUNT
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BlobsBundleError {
    #[error("Blob data has an invalid length")]
    BlobDataInvalidBytesLength,
    #[error("Blob bundle is empty")]
    BlobBundleEmptyError,
    #[error("Blob versioned hashes and blobs bundle content length mismatch")]
    BlobsBundleWrongLen,
    #[error("Blob versioned hashes are incorrect")]
    BlobVersionedHashesError,
    #[error("Blob to commitment and proof generation error")]
    BlobToCommitmentAndProofError,
    #[error("Max blobs per block exceeded")]
    MaxBlobsExceeded,
    #[error("Invalid blob version for the current fork")]
    InvalidBlobVersionForFork,
    #[cfg(feature = "c-kzg")]
    #[error("KZG related error: {0}")]
    Kzg(#[from] ethrex_crypto::kzg::KzgError),
}

#[cfg(test)]
mod tests {
    mod shared {
        pub fn dummy_v1_bundle(blob_count: usize) -> crate::types::BlobsBundle {
            crate::types::BlobsBundle {
                blobs: vec![[0; crate::types::BYTES_PER_BLOB]; blob_count],
                commitments: vec![[0; 48]; blob_count],
                proofs: vec![[0; 48]; blob_count * crate::types::CELLS_PER_EXT_BLOB],
                version: 1,
            }
        }

        #[cfg(feature = "c-kzg")]
        pub fn convert_str_to_bytes48(s: &str) -> [u8; 48] {
            let bytes = hex::decode(s).expect("Invalid hex string");
            let mut array = [0u8; 48];
            array.copy_from_slice(&bytes[..48]);
            array
        }
    }

    #[test]
    fn append_v1_sets_the_accumulator_version() {
        let mut aggregate = crate::types::BlobsBundle::empty();
        let first = shared::dummy_v1_bundle(1);
        let second = shared::dummy_v1_bundle(1);

        aggregate.append_v1(first).expect("valid first bundle");
        aggregate.append_v1(second).expect("valid second bundle");

        assert_eq!(aggregate.version, 1);
        assert_eq!(aggregate.blobs.len(), 2);
        assert_eq!(aggregate.commitments.len(), 2);
        assert_eq!(aggregate.proofs.len(), 2 * crate::types::CELLS_PER_EXT_BLOB);
    }

    #[test]
    fn append_v1_rejects_legacy_or_malformed_bundles() {
        let mut aggregate = shared::dummy_v1_bundle(1);
        let original = aggregate.clone();
        let mut legacy = shared::dummy_v1_bundle(1);
        legacy.version = 0;

        assert!(matches!(
            aggregate.append_v1(legacy),
            Err(crate::types::BlobsBundleError::InvalidBlobVersionForFork)
        ));
        assert_eq!(aggregate, original);

        let mut malformed = shared::dummy_v1_bundle(1);
        malformed.proofs.pop();
        assert!(matches!(
            aggregate.append_v1(malformed),
            Err(crate::types::BlobsBundleError::BlobsBundleWrongLen)
        ));
        assert_eq!(aggregate, original);
    }

    #[test]
    fn empty_v1_bundle_is_structurally_valid_for_validium() {
        let bundle = crate::types::BlobsBundle::default();
        assert_eq!(bundle.version, 1);
        assert!(bundle.validate_v1_structure().is_ok());
    }

    #[test]
    #[cfg(feature = "c-kzg")]
    fn invalid_v1_cell_proofs_are_rejected() {
        let bundle = shared::dummy_v1_bundle(1);
        assert!(bundle.verify_kzg_proofs().is_err());
    }

    #[test]
    #[cfg(feature = "c-kzg")]
    fn blob_to_kzg_proof_verifies_for_prover_path() {
        let blob = crate::types::blobs_bundle::blob_from_bytes("prover blob".as_bytes().into())
            .expect("blob");
        let commitment = ethrex_crypto::kzg::blob_to_kzg_commitment_and_proof(&blob)
            .expect("commitment")
            .0;
        let proof = crate::types::blobs_bundle::blob_to_kzg_proof(&blob).expect("single proof");
        assert!(
            ethrex_crypto::kzg::verify_blob_kzg_proof(blob, commitment, proof).expect("verify"),
            "committer prover path must produce a valid EIP-4844 proof"
        );
    }

    #[test]
    #[cfg(feature = "c-kzg")]
    fn create_from_blobs_regenerates_v1_cell_proof_layout() {
        let blob = crate::types::blobs_bundle::blob_from_bytes("stored blob".as_bytes().into())
            .expect("blob");
        let bundle = crate::types::BlobsBundle::create_from_blobs(&vec![blob])
            .expect("regenerate like Store::get_batch");
        assert_eq!(bundle.version, 1);
        assert_eq!(bundle.proofs.len(), crate::types::CELLS_PER_EXT_BLOB);
        assert!(bundle.validate_v1_structure().is_ok());
        assert!(bundle.verify_kzg_proofs().is_ok());
    }

    #[test]
    #[cfg(feature = "c-kzg")]
    fn transaction_with_valid_blobs_should_pass() {
        let blobs = vec!["Hello, world!".as_bytes(), "Goodbye, world!".as_bytes()]
            .into_iter()
            .map(|data| {
                crate::types::blobs_bundle::blob_from_bytes(data.into())
                    .expect("Failed to create blob")
            })
            .collect();

        let blobs_bundle = crate::types::BlobsBundle::create_from_blobs(&blobs)
            .expect("Failed to create blobs bundle");

        let blob_versioned_hashes = blobs_bundle.generate_versioned_hashes();

        let tx = crate::types::transaction::EIP4844Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: 0.into(),
            gas: 15_000_000,
            to: crate::Address::from_low_u64_be(1), // Normal tx
            value: crate::U256::zero(),             // Value zero
            data: crate::Bytes::default(),          // No data
            access_list: Default::default(),        // No access list
            blob_versioned_hashes,
            ..Default::default()
        };

        assert!(matches!(
            blobs_bundle.validate(&tx, crate::types::Fork::Prague),
            Ok(())
        ));
    }

    #[test]
    #[cfg(feature = "c-kzg")]
    fn transaction_with_valid_blobs_should_pass_on_osaka() {
        let blobs = vec!["Hello, world!".as_bytes(), "Goodbye, world!".as_bytes()]
            .into_iter()
            .map(|data| {
                crate::types::blobs_bundle::blob_from_bytes(data.into())
                    .expect("Failed to create blob")
            })
            .collect();

        let blobs_bundle = crate::types::BlobsBundle::create_from_blobs(&blobs)
            .expect("Failed to create blobs bundle");

        let blob_versioned_hashes = blobs_bundle.generate_versioned_hashes();

        let tx = crate::types::transaction::EIP4844Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: 0.into(),
            gas: 15_000_000,
            to: crate::Address::from_low_u64_be(1), // Normal tx
            value: crate::U256::zero(),             // Value zero
            data: crate::Bytes::default(),          // No data
            access_list: Default::default(),        // No access list
            blob_versioned_hashes,
            ..Default::default()
        };

        assert!(matches!(
            blobs_bundle.validate(&tx, crate::types::Fork::Osaka),
            Ok(())
        ));
    }

    // v1 (EIP-7594 cell-proof) is now the only accepted sidecar version at all forks,
    // following go-ethereum#35191.
    #[test]
    #[cfg(feature = "c-kzg")]
    fn v1_sidecar_is_accepted_at_all_forks() {
        let blobs = vec!["Hello, world!".as_bytes(), "Goodbye, world!".as_bytes()]
            .into_iter()
            .map(|data| {
                crate::types::blobs_bundle::blob_from_bytes(data.into())
                    .expect("Failed to create blob")
            })
            .collect();

        let blobs_bundle = crate::types::BlobsBundle::create_from_blobs(&blobs)
            .expect("Failed to create blobs bundle");

        let blob_versioned_hashes = blobs_bundle.generate_versioned_hashes();

        let tx = crate::types::transaction::EIP4844Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: 0.into(),
            gas: 15_000_000,
            to: crate::Address::from_low_u64_be(1), // Normal tx
            value: crate::U256::zero(),             // Value zero
            data: crate::Bytes::default(),          // No data
            access_list: Default::default(),        // No access list
            blob_versioned_hashes,
            ..Default::default()
        };

        assert!(matches!(
            blobs_bundle.validate(&tx, crate::types::Fork::Prague),
            Ok(())
        ));
    }

    #[test]
    #[cfg(feature = "c-kzg")]
    fn v0_sidecar_is_rejected_at_all_forks() {
        // v0 (EIP-4844 KZG-proof) sidecars are no longer valid at any fork,
        // following go-ethereum#35191. Only v1 (EIP-7594 cell-proof) is accepted.
        let blobs = vec!["Hello, world!".as_bytes(), "Goodbye, world!".as_bytes()]
            .into_iter()
            .map(|data| {
                crate::types::blobs_bundle::blob_from_bytes(data.into())
                    .expect("Failed to create blob")
            })
            .collect::<Vec<_>>();

        // Build a v1 bundle to get valid commitments/hashes, then manually construct
        // a v0-shaped bundle (one proof per blob, version 0).
        let v1 = crate::types::BlobsBundle::create_from_blobs(&blobs)
            .expect("Failed to create blobs bundle");
        let blob_versioned_hashes = v1.generate_versioned_hashes();
        let v0_bundle = crate::types::BlobsBundle {
            blobs: v1.blobs,
            commitments: v1.commitments,
            proofs: vec![[0u8; 48]; blobs.len()], // one dummy proof per blob (v0 shape)
            version: 0,
        };

        let tx = crate::types::transaction::EIP4844Transaction {
            blob_versioned_hashes,
            ..Default::default()
        };

        assert!(matches!(
            v0_bundle.validate_cheap(&tx, crate::types::Fork::Prague),
            Err(crate::types::BlobsBundleError::InvalidBlobVersionForFork)
        ));
        assert!(matches!(
            v0_bundle.validate_cheap(&tx, crate::types::Fork::Osaka),
            Err(crate::types::BlobsBundleError::InvalidBlobVersionForFork)
        ));
    }

    #[test]
    #[cfg(feature = "c-kzg")]
    fn transaction_with_invalid_proofs_should_fail() {
        // blob data taken from: https://etherscan.io/tx/0x02a623925c05c540a7633ffa4eb78474df826497faa81035c4168695656801a2#blobs, but with 0 size blobs
        let blobs_bundle = crate::types::BlobsBundle {
            blobs: vec![[0; crate::types::BYTES_PER_BLOB], [0; crate::types::BYTES_PER_BLOB]],
            commitments: vec!["b90289aabe0fcfb8db20a76b863ba90912d1d4d040cb7a156427d1c8cd5825b4d95eaeb221124782cc216960a3d01ec5",
                              "91189a03ce1fe1225fc5de41d502c3911c2b19596f9011ea5fca4bf311424e5f853c9c46fe026038036c766197af96a0"]
                              .into_iter()
                              .map(|s| {
                                  shared::convert_str_to_bytes48(s)
                              })
                              .collect(),
            proofs: vec!["b502263fc5e75b3587f4fb418e61c5d0f0c18980b4e00179326a65d082539a50c063507a0b028e2db10c55814acbe4e9",
                         "a29c43f6d05b7f15ab6f3e5004bd5f6b190165dc17e3d51fd06179b1e42c7aef50c145750d7c1cd1cd28357593bc7658"]
                            .into_iter()
                            .map(|s| {
                                shared::convert_str_to_bytes48(s)
                            })
                            .collect(),
                            // v0 is now rejected at the version check before KZG verification.
                            version: 0,
        };

        let tx = crate::types::transaction::EIP4844Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: 0.into(),
            gas: 15_000_000,
            to: crate::Address::from_low_u64_be(1), // Normal tx
            value: crate::U256::zero(),             // Value zero
            data: crate::Bytes::default(),          // No data
            access_list: Default::default(),        // No access list
            blob_versioned_hashes: vec![
                "01ec8054d05bfec80f49231c6e90528bbb826ccd1464c255f38004099c8918d9",
                "0180cb2dee9e6e016fabb5da4fb208555f5145c32895ccd13b26266d558cd77d",
            ]
            .into_iter()
            .map(|b| {
                let bytes = hex::decode(b).expect("Invalid hex string");
                crate::H256::from_slice(&bytes)
            })
            .collect::<Vec<crate::H256>>(),
            ..Default::default()
        };

        assert!(matches!(
            blobs_bundle.validate(&tx, crate::types::Fork::Prague),
            Err(crate::types::BlobsBundleError::InvalidBlobVersionForFork)
        ));
    }

    #[test]
    #[cfg(feature = "c-kzg")]
    fn transaction_with_incorrect_blobs_should_fail() {
        // blob data taken from: https://etherscan.io/tx/0x02a623925c05c540a7633ffa4eb78474df826497faa81035c4168695656801a2#blobs
        let blobs_bundle = crate::types::BlobsBundle {
            blobs: vec![[0; crate::types::BYTES_PER_BLOB], [0; crate::types::BYTES_PER_BLOB]],
            commitments: vec!["dead89aabe0fcfb8db20a76b863ba90912d1d4d040cb7a156427d1c8cd5825b4d95eaeb221124782cc216960a3d01ec5",
                              "91189a03ce1fe1225fc5de41d502c3911c2b19596f9011ea5fca4bf311424e5f853c9c46fe026038036c766197af96a0"]
                              .into_iter()
                              .map(|s| {
                                shared::convert_str_to_bytes48(s)
                              })
                              .collect(),
            proofs: vec!["b502263fc5e75b3587f4fb418e61c5d0f0c18980b4e00179326a65d082539a50c063507a0b028e2db10c55814acbe4e9",
                         "a29c43f6d05b7f15ab6f3e5004bd5f6b190165dc17e3d51fd06179b1e42c7aef50c145750d7c1cd1cd28357593bc7658"]
                         .into_iter()
                              .map(|s| {
                                shared::convert_str_to_bytes48(s)
                              })
                              .collect(),
                              // v0 is now rejected at the version check before commitment hash verification.
                              version: 0,
        };

        let tx = crate::types::transaction::EIP4844Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: 0.into(),
            gas: 15_000_000,
            to: crate::Address::from_low_u64_be(1), // Normal tx
            value: crate::U256::zero(),             // Value zero
            data: crate::Bytes::default(),          // No data
            access_list: Default::default(),        // No access list
            blob_versioned_hashes: vec![
                "01ec8054d05bfec80f49231c6e90528bbb826ccd1464c255f38004099c8918d9",
                "0180cb2dee9e6e016fabb5da4fb208555f5145c32895ccd13b26266d558cd77d",
            ]
            .into_iter()
            .map(|b| {
                let bytes = hex::decode(b).expect("Invalid hex string");
                crate::H256::from_slice(&bytes)
            })
            .collect::<Vec<crate::H256>>(),
            ..Default::default()
        };

        assert!(matches!(
            blobs_bundle.validate(&tx, crate::types::Fork::Prague),
            Err(crate::types::BlobsBundleError::InvalidBlobVersionForFork)
        ));
    }

    #[test]
    #[cfg(feature = "c-kzg")]
    fn transaction_with_too_many_blobs_should_fail() {
        let blob = crate::types::blobs_bundle::blob_from_bytes("Im a Blob".as_bytes().into())
            .expect("Failed to create blob");
        let blobs =
            std::iter::repeat_n(blob, super::MAX_BLOB_COUNT_ELECTRA + 1).collect::<Vec<_>>();

        let blobs_bundle = crate::types::BlobsBundle::create_from_blobs(&blobs)
            .expect("Failed to create blobs bundle");

        let blob_versioned_hashes = blobs_bundle.generate_versioned_hashes();

        let tx = crate::types::transaction::EIP4844Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: 0.into(),
            gas: 15_000_000,
            to: crate::Address::from_low_u64_be(1), // Normal tx
            value: crate::U256::zero(),             // Value zero
            data: crate::Bytes::default(),          // No data
            access_list: Default::default(),        // No access list
            blob_versioned_hashes,
            ..Default::default()
        };

        assert!(matches!(
            blobs_bundle.validate(&tx, crate::types::Fork::Prague),
            Err(crate::types::BlobsBundleError::MaxBlobsExceeded)
        ));
    }

    #[test]
    #[cfg(feature = "c-kzg")]
    fn transaction_with_version_0_blobs_should_fail_on_all_forks() {
        // v0 blobs are rejected at every fork now that only v1 (cell-proof) sidecars
        // are accepted, following go-ethereum#35191.
        let blobs = vec!["Hello, world!".as_bytes(), "Goodbye, world!".as_bytes()]
            .into_iter()
            .map(|data| {
                crate::types::blobs_bundle::blob_from_bytes(data.into())
                    .expect("Failed to create blob")
            })
            .collect::<Vec<_>>();

        // Build a v1 bundle to obtain valid commitments and versioned hashes, then
        // manually downgrade to version 0 to simulate a legacy v0 sidecar.
        let v1 = crate::types::BlobsBundle::create_from_blobs(&blobs)
            .expect("Failed to create blobs bundle");
        let blob_versioned_hashes = v1.generate_versioned_hashes();
        let v0_bundle = crate::types::BlobsBundle {
            blobs: v1.blobs,
            commitments: v1.commitments,
            proofs: vec![[0u8; 48]; blobs.len()],
            version: 0,
        };

        let tx = crate::types::transaction::EIP4844Transaction {
            blob_versioned_hashes,
            ..Default::default()
        };

        for fork in [
            crate::types::Fork::Prague,
            crate::types::Fork::Osaka,
            crate::types::Fork::Amsterdam,
        ] {
            assert!(
                matches!(
                    v0_bundle.validate_cheap(&tx, fork),
                    Err(crate::types::BlobsBundleError::InvalidBlobVersionForFork)
                ),
                "v0 bundle should be rejected on fork {:?}",
                fork
            );
        }
    }
}
