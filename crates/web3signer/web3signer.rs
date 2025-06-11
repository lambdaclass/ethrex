use bytes::Bytes;
use ethereum_types::Signature;
use reqwest::Client;
use rustc_hex::FromHexError;
use secp256k1::PublicKey;

#[derive(Debug, thiserror::Error)]
pub enum WebsignError {
    #[error("Failed with a reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("Failed to parse value: {0}")]
    FromHexError(#[from] FromHexError),
}

pub async fn web3sign(
    data: Bytes,
    url: String,
    public_key: PublicKey,
) -> Result<Signature, WebsignError> {
    let url = format!(
        "{}api/v1/eth1/sign/{}",
        url,
        hex::encode(&public_key.serialize_uncompressed()[1..]),
    );
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
