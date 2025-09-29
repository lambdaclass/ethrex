use ethrex_common::Address;
use ethrex_common::utils::keccak;

/// secp256k1 > k256
#[allow(clippy::needless_return)]
pub fn get_address_from_secret_key(secret_key: &secp256k1::SecretKey) -> Result<Address, String> {
    #[cfg(all(
        not(feature = "secp256k1"),
        not(feature = "k256")
    ))]
    {
        compile_error!(
            "Either the `secp256k1` or `k256` feature must be enabled to use KZG functionality."
        );
        return Ok(false);
    }
    #[cfg(feature = "secp256k1")]
    {
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

        return Ok(Address::from(address_bytes));
    }
    #[cfg(feature = "k256")]
    {
        use k256::elliptic_curve::sec1::ToEncodedPoint;

        let public_key = secret_key.public_key().to_encoded_point(false);
        let hash = keccak(
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

        return Ok(Address::from(address_bytes));
    }
}
