// Compile-time check to ensure at least one backend feature is enabled
#[cfg(not(any(feature = "secp256k1", feature = "k256")))]
const _: () = {
    compile_error!(
        "Either the `secp256k1` or `k256` feature must be enabled to use get_address_from_secret_key."
    );
};

// Compile-time check to ensure exactly one backend feature is enabled
#[cfg(all(feature = "secp256k1", feature = "k256"))]
const _: () = {
    compile_error!(
        "Either the `secp256k1` or `k256` feature must be enabled to use get_address_from_secret_key."
    );
};

#[expect(clippy::needless_return)]
pub fn get_address_from_secret_key(
    secret_key_bytes: &[u8],
) -> Result<ethrex_common::Address, String> {
    #[cfg(feature = "secp256k1")]
    {
        let secret_key = secp256k1::SecretKey::from_slice(secret_key_bytes)
            .map_err(|e| format!("Failed to parse secret key from slice: {e}"))?;
        return get_address_from_secret_key_secp256(secret_key);
    }

    #[cfg(all(not(feature = "secp256k1"), feature = "k256"))]
    {
        let secret_key = k256::SecretKey::from_slice(secret_key_bytes)
            .map_err(|e| format!("Failed to parse secret key from slice: {e}"))?;
        return get_address_from_secret_key_k256(secret_key);
    }
    Ok(ethrex_common::Address::default()) // FIXME
}

#[cfg(feature = "secp256k1")]
pub fn get_address_from_secret_key_secp256(
    secret_key: secp256k1::SecretKey,
) -> Result<ethrex_common::Address, String> {
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

#[cfg(feature = "k256")]
pub fn get_address_from_secret_key_k256(
    secret_key: k256::SecretKey,
) -> Result<ethrex_common::Address, String> {
    use k256::elliptic_curve::sec1::ToEncodedPoint;

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
