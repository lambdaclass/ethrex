use ethrex_common::{types::Transaction, Address, H256};
use keccak_hash::keccak;
use secp256k1::SecretKey;

/// Returns the formatted hash of the withdrawal transaction,
/// or None if the transaction is not a withdrawal.
/// The hash is computed as keccak256(to || value || tx_hash)
pub fn get_withdrawal_hash(tx: &Transaction) -> Option<H256> {
    let to_bytes: [u8; 20] = match tx.data().get(16..36)?.try_into() {
        Ok(value) => value,
        Err(_) => return None,
    };
    let to = Address::from(to_bytes);

    let value = tx.value().to_big_endian();

    Some(keccak_hash::keccak(
        [to.as_bytes(), &value, tx.compute_hash().as_bytes()].concat(),
    ))
}

pub fn merkle_proof(data: Vec<H256>, base_element: H256) -> Option<Vec<H256>> {
    use keccak_hash::keccak;

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
            .flat_map(|chunk| -> Option<H256> {
                let left = chunk.first().copied()?;

                let right = chunk.get(1).copied().unwrap_or(left);
                let result = keccak([left.as_bytes(), right.as_bytes()].concat())
                    .as_fixed_bytes()
                    .into();
                if left == current_target {
                    proof.push(right);
                    target_hash = result;
                } else if right == current_target {
                    proof.push(left);
                    target_hash = result;
                }
                Some(result)
            })
            .collect();
    }
    Some(proof)
}

pub fn get_address_from_secret_key(secret_key: &SecretKey) -> Option<Address> {
    let public_key = secret_key
        .public_key(secp256k1::SECP256K1)
        .serialize_uncompressed();
    let hash = keccak(&public_key[1..]);
    let address_bytes: [u8; 20] = hash.as_ref().get(12..32)?.try_into().ok()?;
    Some(Address::from(address_bytes))
}
