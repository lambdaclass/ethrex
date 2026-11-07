use bytes::Bytes;
use ethrex_common::H256;
use ethrex_common::types::fee_config::FeeConfig;
use ethrex_common::types::{
    Block, Commitment, Proof, blob_from_bytes, kzg_commitment_to_versioned_hash,
};
use ethrex_crypto::Crypto;
use ethrex_rlp::encode::RLPEncode;

use crate::l2::L2ExecutionError;

/// Verify the KZG blob proof and return the versioned hash.
///
/// Returns `H256::zero()` for validium mode (when commitment and proof are all zeros).
///
/// The verification goes through the `Crypto` provider so each zkVM can bring
/// its own KZG backend: the defaults use the crate's `kzg-rs`/`c-kzg` build, and
/// ZisK combines its point-evaluation accelerator with portable field arithmetic.
pub fn verify_blob(
    blocks: &[Block],
    fee_configs: &[FeeConfig],
    commitment: Commitment,
    proof: Proof,
    crypto: &dyn Crypto,
) -> Result<H256, L2ExecutionError> {
    // Check for validium mode (no blob data)
    let validium = (commitment, &proof) == ([0; 48], &[0; 48]);
    if validium {
        return Ok(H256::zero());
    }

    let len: u64 = blocks.len().try_into()?;
    let mut blob_data = Vec::new();

    blob_data.extend(len.to_be_bytes());

    for block in blocks {
        blob_data.extend(block.encode_to_vec());
    }

    for fee_config in fee_configs {
        blob_data.extend(fee_config.to_vec());
    }

    let blob_data = blob_from_bytes(Bytes::from(blob_data))?;

    let valid = crypto
        .verify_blob_kzg_proof(&blob_data, &commitment, &proof)
        .map_err(|e| L2ExecutionError::Internal(format!("blob KZG verification: {e}")))?;
    if !valid {
        return Err(L2ExecutionError::InvalidBlobProof);
    }

    Ok(kzg_commitment_to_versioned_hash(&commitment))
}
