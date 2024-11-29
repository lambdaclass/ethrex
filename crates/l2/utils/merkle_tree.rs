use keccak_hash::{keccak, H256};
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Debug, thiserror::Error, Clone, Serialize, Deserialize)]
pub enum MerkleError {
    #[error("Left element is None")]
    LeftElementIsNone(),
    #[error("Data vector is empty")]
    DataVectorIsEmpty(),
}

pub fn merkelize(data: Vec<H256>) -> Result<H256, MerkleError> {
    info!("Merkelizing {:?}", data);
    let mut data = data;
    let mut first = true;
    while data.len() > 1 || first {
        first = false;
        data = data
            .chunks(2)
            .flat_map(|chunk| -> Result<H256, MerkleError> {
                let left = chunk.first().ok_or(MerkleError::LeftElementIsNone())?;
                let right = *chunk.get(1).unwrap_or(left);
                Ok(keccak([left.as_bytes(), right.as_bytes()].concat()))
            })
            .collect();
    }
    data.first()
        .copied()
        .ok_or(MerkleError::DataVectorIsEmpty())
}

pub fn merkle_proof(data: Vec<H256>, base_element: H256) -> Result<Option<Vec<H256>>, MerkleError> {
    if !data.contains(&base_element) {
        return Ok(None);
    }

    let mut proof = vec![];
    let mut data = data;

    let mut target_hash = base_element;
    let mut first = true;
    while data.len() > 1 || first {
        first = false;
        let current_target = target_hash;
        data = data
            .chunks(2)
            .flat_map(|chunk| -> Result<H256, MerkleError> {
                let left = chunk
                    .first()
                    .copied()
                    .ok_or(MerkleError::LeftElementIsNone())?;
                let right = chunk.get(1).copied().unwrap_or(left);
                let result = keccak([left.as_bytes(), right.as_bytes()].concat());
                if left == current_target {
                    proof.push(right);
                    target_hash = result;
                } else if right == current_target {
                    proof.push(left);
                    target_hash = result;
                }
                Ok(result)
            })
            .collect();
    }

    Ok(Some(proof))
}
