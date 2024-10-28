use keccak_hash::{keccak, H256};
use tracing::info;

pub fn merkelize(data: Vec<H256>) -> H256 {
    info!("Merkelizing {:?}", data);
    let mut data = data;
    let mut first = true;
    while data.len() > 1 || first {
        first = false;
        data = data
            .chunks(2)
            .map(|chunk| {
                let left = chunk[0];
                let right = *chunk.get(1).unwrap_or(&left);
                keccak([left.as_bytes(), right.as_bytes()].concat())
            })
            .collect();
    }
    data[0]
}

pub fn merkle_proof(data: Vec<H256>, base_element: H256) -> Option<Vec<H256>> {
    if !data.contains(&base_element) {
        return None;
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
            .map(|chunk| {
                let left = chunk[0];
                let right = *chunk.get(1).unwrap_or(&left);
                let result = keccak([left.as_bytes(), right.as_bytes()].concat());
                if left == current_target {
                    proof.push(right);
                    target_hash = result;
                } else if right == current_target {
                    proof.push(left);
                    target_hash = result;
                }
                result
            })
            .collect();
    }

    Some(proof)
}
