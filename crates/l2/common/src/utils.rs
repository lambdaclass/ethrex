use ethrex_common::Address;
use ethrex_common::utils::keccak;
use secp256k1::SecretKey;

pub fn get_address_from_secret_key(secret_key: &SecretKey) -> Result<Address, String> {
    let public_key = secret_key
        .public_key(secp256k1::SECP256K1)
        .serialize_uncompressed();
    let hash = keccak(&public_key[1..]);

    // Get the last 20 bytes of the hash
    let address_bytes: [u8; 20] = hash
        .as_ref()
        .get(12..32)
        .ok_or("Failed to get_address_from_secret_key: error slicing address_bytes".to_owned())?
        .try_into()
        .map_err(|err| format!("Failed to get_address_from_secret_key: {err}"))?;

    Ok(Address::from(address_bytes))
}
