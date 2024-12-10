#[cfg(feature = "c-kzg")]
use lazy_static::lazy_static;
use std::ops::AddAssign;

use crate::serde_utils;
use crate::{
    types::{constants::VERSIONED_HASH_VERSION_KZG, transaction::EIP4844Transaction},
    Bytes, H256,
};

#[cfg(feature = "c-kzg")]
use c_kzg::{ethereum_kzg_settings, KzgCommitment, KzgProof, KzgSettings};

use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use serde::{Deserialize, Serialize};

use super::BYTES_PER_BLOB;

pub type Bytes48 = [u8; 48];
pub type Blob = [u8; BYTES_PER_BLOB];
pub type Commitment = Bytes48;
pub type Proof = Bytes48;

#[cfg(feature = "c-kzg")]
lazy_static! {
    static ref KZG_SETTINGS: &'static KzgSettings = ethereum_kzg_settings();
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
/// Struct containing all the blobs for a blob transaction, along with the corresponding commitments and proofs
pub struct BlobsBundle {
    #[serde(with = "serde_utils::blob::vec")]
    pub blobs: Vec<Blob>,
    #[serde(with = "serde_utils::bytes48::vec")]
    pub commitments: Vec<Commitment>,
    #[serde(with = "serde_utils::bytes48::vec")]
    pub proofs: Vec<Proof>,
}

pub fn blob_from_bytes(bytes: Bytes) -> Result<Blob, BlobsBundleError> {
    // This functions moved from `l2/utils/eth_client/transaction.rs`
    // We set the first byte of every 32-bytes chunk to 0x00
    // so it's always under the field module.
    if bytes.len() > BYTES_PER_BLOB * 31 / 32 {
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

fn kzg_commitment_to_versioned_hash(data: &Commitment) -> H256 {
    use k256::sha2::Digest;
    let mut versioned_hash: [u8; 32] = k256::sha2::Sha256::digest(data).into();
    versioned_hash[0] = VERSIONED_HASH_VERSION_KZG;
    versioned_hash.into()
}

#[cfg(feature = "c-kzg")]
fn blob_to_kzg_commitment_and_proof(blob: &Blob) -> Result<(Commitment, Proof), BlobsBundleError> {
    let blob: c_kzg::Blob = (*blob).into();

    let commitment = KzgCommitment::blob_to_kzg_commitment(&blob, &KZG_SETTINGS)
        .or(Err(BlobsBundleError::BlobToCommitmentAndProofError))?;

    let commitment_bytes = commitment.to_bytes();

    let proof = KzgProof::compute_blob_kzg_proof(&blob, &commitment_bytes, &KZG_SETTINGS)
        .or(Err(BlobsBundleError::BlobToCommitmentAndProofError))?;

    let proof_bytes = proof.to_bytes();

    Ok((commitment_bytes.into_inner(), proof_bytes.into_inner()))
}

#[cfg(feature = "c-kzg")]
fn verify_blob_kzg_proof(
    blob: Blob,
    commitment: Commitment,
    proof: Proof,
) -> Result<bool, BlobsBundleError> {
    let blob: c_kzg::Blob = blob.into();
    let commitment: c_kzg::Bytes48 = commitment.into();
    let proof: c_kzg::Bytes48 = proof.into();

    KzgProof::verify_blob_kzg_proof(&blob, &commitment, &proof, &KZG_SETTINGS)
        .or(Err(BlobsBundleError::BlobToCommitmentAndProofError))
}

impl BlobsBundle {
    pub fn empty() -> Self {
        Self::default()
    }

    // In the future we might want to provide a new method that calculates the commitments and proofs using the following.
    #[cfg(feature = "c-kzg")]
    pub fn create_from_blobs(blobs: &Vec<Blob>) -> Result<Self, BlobsBundleError> {
        let mut commitments = Vec::new();
        let mut proofs = Vec::new();

        // Populate the commitments and proofs
        for blob in blobs {
            let (commitment, proof) = blob_to_kzg_commitment_and_proof(blob)?;
            commitments.push(commitment);
            proofs.push(proof);
        }

        Ok(Self {
            blobs: blobs.clone(),
            commitments,
            proofs,
        })
    }

    pub fn generate_versioned_hashes(&self) -> Vec<H256> {
        self.commitments
            .iter()
            .map(kzg_commitment_to_versioned_hash)
            .collect()
    }

    #[cfg(feature = "c-kzg")]
    pub fn validate(&self, tx: &EIP4844Transaction) -> Result<(), BlobsBundleError> {
        let blob_count = self.blobs.len();

        // Check if the blob bundle is empty
        if blob_count == 0 {
            return Err(BlobsBundleError::BlobBundleEmptyError);
        }

        // Check if the blob versioned hashes and blobs bundle content length mismatch
        if blob_count != self.commitments.len()
            || blob_count != self.proofs.len()
            || blob_count != tx.blob_versioned_hashes.len()
        {
            return Err(BlobsBundleError::BlobsBundleWrongLen);
        };

        // Check versioned hashes match the tx
        for (commitment, blob_versioned_hash) in
            self.commitments.iter().zip(tx.blob_versioned_hashes.iter())
        {
            if *blob_versioned_hash != kzg_commitment_to_versioned_hash(commitment) {
                return Err(BlobsBundleError::BlobVersionedHashesError);
            }
        }

        // Validate the blobs with the commitments and proofs
        for ((blob, commitment), proof) in self
            .blobs
            .iter()
            .zip(self.commitments.iter())
            .zip(self.proofs.iter())
        {
            if !verify_blob_kzg_proof(*blob, *commitment, *proof)? {
                return Err(BlobsBundleError::BlobToCommitmentAndProofError);
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
            .finish();
    }
}

impl RLPDecode for BlobsBundle {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (blobs, decoder) = decoder.decode_field("blobs")?;
        let (commitments, decoder) = decoder.decode_field("commitments")?;
        let (proofs, decoder) = decoder.decode_field("proofs")?;
        Ok((
            Self {
                blobs,
                commitments,
                proofs,
            },
            decoder.finish()?,
        ))
    }
}

impl AddAssign for BlobsBundle {
    fn add_assign(&mut self, rhs: Self) {
        self.blobs.extend_from_slice(&rhs.blobs);
        self.commitments.extend_from_slice(&rhs.commitments);
        self.proofs.extend_from_slice(&rhs.proofs);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        types::{blobs_bundle, transaction::EIP4844Transaction},
        Address, Bytes, U256,
    };
    mod shared {
        pub fn convert_str_to_bytes48(s: &str) -> [u8; 48] {
            let bytes = hex::decode(s).expect("Invalid hex string");
            let mut array = [0u8; 48];
            array.copy_from_slice(&bytes[..48]);
            array
        }
    }

    #[test]
    fn transaction_with_valid_blobs_should_pass() {
        let blobs = vec!["Hello, world!".as_bytes(), "Goodbye, world!".as_bytes()]
            .into_iter()
            .map(|data| blobs_bundle::blob_from_bytes(data.into()).expect("Failed to create blob"))
            .collect();

        let blobs_bundle =
            BlobsBundle::create_from_blobs(&blobs).expect("Failed to create blobs bundle");

        let blob_versioned_hashes = blobs_bundle.generate_versioned_hashes();

        let tx = EIP4844Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: 0.into(),
            gas: 15_000_000,
            to: Address::from_low_u64_be(1), // Normal tx
            value: U256::zero(),             // Value zero
            data: Bytes::default(),          // No data
            access_list: Default::default(), // No access list
            blob_versioned_hashes,
            ..Default::default()
        };

        assert!(matches!(blobs_bundle.validate(&tx), Ok(())));
    }
    #[test]
    fn transaction_with_invalid_proofs_should_fail() {
        // blob data taken from: https://etherscan.io/tx/0x02a623925c05c540a7633ffa4eb78474df826497faa81035c4168695656801a2#blobs, but with 0 size blobs
        let blobs_bundle = BlobsBundle {
            blobs: vec![[0; BYTES_PER_BLOB], [0; BYTES_PER_BLOB]],
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
                            .collect()
        };

        let tx = EIP4844Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: 0.into(),
            gas: 15_000_000,
            to: Address::from_low_u64_be(1), // Normal tx
            value: U256::zero(),             // Value zero
            data: Bytes::default(),          // No data
            access_list: Default::default(), // No access list
            blob_versioned_hashes: vec![
                "01ec8054d05bfec80f49231c6e90528bbb826ccd1464c255f38004099c8918d9",
                "0180cb2dee9e6e016fabb5da4fb208555f5145c32895ccd13b26266d558cd77d",
            ]
            .into_iter()
            .map(|b| {
                let bytes = hex::decode(b).expect("Invalid hex string");
                H256::from_slice(&bytes)
            })
            .collect::<Vec<H256>>(),
            ..Default::default()
        };

        assert!(matches!(
            blobs_bundle.validate(&tx),
            Err(BlobsBundleError::BlobToCommitmentAndProofError)
        ));
    }

    #[test]
    fn transaction_with_incorrect_blobs_should_fail() {
        // blob data taken from: https://etherscan.io/tx/0x02a623925c05c540a7633ffa4eb78474df826497faa81035c4168695656801a2#blobs
        let blobs_bundle = BlobsBundle {
            blobs: vec![[0; BYTES_PER_BLOB], [0; BYTES_PER_BLOB]],
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
                              .collect()
        };

        let tx = EIP4844Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: 0.into(),
            gas: 15_000_000,
            to: Address::from_low_u64_be(1), // Normal tx
            value: U256::zero(),             // Value zero
            data: Bytes::default(),          // No data
            access_list: Default::default(), // No access list
            blob_versioned_hashes: vec![
                "01ec8054d05bfec80f49231c6e90528bbb826ccd1464c255f38004099c8918d9",
                "0180cb2dee9e6e016fabb5da4fb208555f5145c32895ccd13b26266d558cd77d",
            ]
            .into_iter()
            .map(|b| {
                let bytes = hex::decode(b).expect("Invalid hex string");
                H256::from_slice(&bytes)
            })
            .collect::<Vec<H256>>(),
            ..Default::default()
        };

        assert!(matches!(
            blobs_bundle.validate(&tx),
            Err(BlobsBundleError::BlobVersionedHashesError)
        ));
    }
}
