#[cfg(all(
    not(feature = "zisk"),
    not(feature = "risc0"),
    not(feature = "sp1"),
    feature = "secp256k1"
))]
pub fn get_address_from_secret_key(
    secret_key_bytes: &[u8],
) -> Result<ethrex_common::Address, String> {
    let secret_key = secp256k1::SecretKey::from_slice(secret_key_bytes)
        .map_err(|e| format!("Failed to parse secret key from slice: {e}"))?;

    let public_key = secret_key
        .public_key(secp256k1::SECP256K1)
        .serialize_uncompressed();
    let hash = ethrex_common::utils::keccak(&public_key[1..]);

    // Get the last 20 bytes of the hash
    let address_bytes: [u8; 20] = hash
        .as_ref()
        .get(12..32)
        .ok_or("Failed to get_address_from_secret_key: error slicing address_bytes".to_owned())?
        .try_into()
        .map_err(|err| format!("Failed to get_address_from_secret_key: {err}"))?;

    Ok(ethrex_common::Address::from(address_bytes))
}

#[cfg(any(
    feature = "zisk",
    feature = "risc0",
    feature = "sp1",
    not(feature = "secp256k1")
))]
pub fn get_address_from_secret_key(
    secret_key_bytes: &[u8],
) -> Result<ethrex_common::Address, String> {
    use k256::elliptic_curve::sec1::ToEncodedPoint;

    let secret_key = k256::SecretKey::from_slice(secret_key_bytes)
        .map_err(|e| format!("Failed to parse secret key from slice: {e}"))?;

    let public_key = secret_key.public_key().to_encoded_point(false);
    let hash = ethrex_common::utils::keccak(
        public_key
            .as_bytes()
            .get(1..)
            .ok_or(String::from("failed to slice public key"))?,
    );

    // Get the last 20 bytes of the hash
    let address_bytes: [u8; 20] = hash
        .as_ref()
        .get(12..32)
        .ok_or("Failed to get_address_from_secret_key: error slicing address_bytes".to_owned())?
        .try_into()
        .map_err(|err| format!("Failed to get_address_from_secret_key: {err}"))?;

    Ok(ethrex_common::Address::from(address_bytes))
}
