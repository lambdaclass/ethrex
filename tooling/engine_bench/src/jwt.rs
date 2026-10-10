use eyre::{Context, Result};
use jsonwebtoken::{EncodingKey, Header, encode};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
struct Claims {
    iat: u64,
}

/// Load a 0x-hex JWT secret file. ethrex stores the secret as a 32-byte hex
/// string at `<datadir>/jwt.hex`.
pub fn load_secret(path: &Path) -> Result<Vec<u8>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading JWT secret from {}", path.display()))?;
    let hex_str = raw.trim().trim_start_matches("0x");
    let bytes = hex::decode(hex_str).context("JWT secret must be valid hex")?;
    if bytes.is_empty() {
        eyre::bail!("JWT secret is empty");
    }
    Ok(bytes)
}

/// Mint a JWT with iat=now and no other claims.
pub fn mint(secret: &[u8]) -> Result<String> {
    let iat = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let token = encode(
        &Header::default(),
        &Claims { iat },
        &EncodingKey::from_secret(secret),
    )?;
    Ok(token)
}
