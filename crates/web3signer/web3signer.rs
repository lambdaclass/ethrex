use bytes::Bytes;
use ethereum_types::Signature;
use reqwest::{Client, Url};
use url::ParseError;
use rustc_hex::FromHexError;
use secp256k1::PublicKey;

#[derive(Debug, thiserror::Error)]
pub enum WebsignError {
    #[error("Url Parse Error: {0}")]
    ParseError(#[from] ParseError),
    #[error("Failed with a reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("Failed to parse value: {0}")]
    FromHexError(#[from] FromHexError),
}

pub async fn web3sign(
    data: Bytes,
    url: Url,
    public_key: PublicKey,
) -> Result<Signature, WebsignError> {
    let url = url
        .join("api/v1/eth/sign")?
        .join(&hex::encode(&public_key.serialize_uncompressed()[1..]))?;
    let body = format!("{{\"data\": \"0x{}\"}}", hex::encode(data));

    let client = Client::new();
    client
        .post(url)
        .body(body)
        .header("content-type", "application/json")
        .send()
        .await?
        .text()
        .await?
        .parse::<Signature>()
        .map_err(WebsignError::FromHexError)
}
