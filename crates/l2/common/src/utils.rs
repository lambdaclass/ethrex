pub fn get_address_from_secret_key(
    secret_key_bytes: &[u8],
) -> Result<ethrex_common::Address, String> {
    let signing_key = k256::ecdsa::SigningKey::from_bytes(secret_key_bytes.into())
        .map_err(|e| format!("Failed to parse secret key from slice: {e}"))?;

    let public_key = signing_key
        .verifying_key()
        .to_encoded_point(false)
        .to_bytes();
    let public_key_without_prefix = public_key
        .get(1..)
        .ok_or("Failed to get_address_from_secret_key: public key too short")?;
    let hash = ethrex_common::utils::keccak(public_key_without_prefix);

    // Get the last 20 bytes of the hash
    let address_bytes: [u8; 20] = hash
        .as_ref()
        .get(12..32)
        .ok_or("Failed to get_address_from_secret_key: error slicing address_bytes".to_owned())?
        .try_into()
        .map_err(|err| format!("Failed to get_address_from_secret_key: {err}"))?;

    Ok(ethrex_common::Address::from(address_bytes))
}
