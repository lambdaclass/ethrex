use bytes::Bytes;
use ethereum_types::{Address, Signature, U256};
use ethrex_rlp::encode::PayloadRLPEncode;
use keccak_hash::keccak;
use reqwest::{Client, Url};
use secp256k1::{Message, PublicKey, SecretKey, SECP256K1};

use crate::types::transaction::{
    EIP1559Transaction, EIP4844Transaction, EIP7702Transaction, TxType,
};

#[derive(Clone, Debug)]
pub enum Signer {
    Local(LocalSigner),
    Remote(RemoteSigner),
}

impl Signer {
    pub async fn sign_eip1559_transaction(
        &self,
        tx: &mut EIP1559Transaction,
    ) -> Result<(), SignerError> {
        let mut payload = tx.encode_payload_to_vec();
        payload.insert(0, TxType::EIP1559.into());

        let signature = self.sign_raw(payload.into()).await?;
        (tx.signature_r, tx.signature_s, tx.signature_y_parity) = parse_signature(signature);

        Ok(())
    }

    pub async fn sign_eip4844_transaction(
        &self,
        tx: &mut EIP4844Transaction,
    ) -> Result<(), SignerError> {
        let mut payload = tx.encode_payload_to_vec();
        payload.insert(0, TxType::EIP4844.into());

        let signature = self.sign_raw(payload.into()).await?;
        (tx.signature_r, tx.signature_s, tx.signature_y_parity) = parse_signature(signature);

        Ok(())
    }

    pub async fn sign_eip7702_transaction(
        &self,
        tx: &mut EIP7702Transaction,
    ) -> Result<(), SignerError> {
        let mut payload = tx.encode_payload_to_vec();
        payload.insert(0, TxType::EIP7702.into());

        let signature = self.sign_raw(payload.into()).await?;
        (tx.signature_r, tx.signature_s, tx.signature_y_parity) = parse_signature(signature);

        Ok(())
    }

    pub async fn sign_raw(&self, data: Bytes) -> Result<Signature, SignerError> {
        match self {
            Self::Local(signer) => Ok(signer.sign(data)),
            Self::Remote(signer) => signer.sign(data).await,
        }
    }

    pub fn address(&self) -> Address {
        match self {
            Self::Local(signer) => signer.address,
            Self::Remote(signer) => signer.address,
        }
    }
}

impl From<LocalSigner> for Signer {
    fn from(value: LocalSigner) -> Self {
        Self::Local(value)
    }
}

impl From<RemoteSigner> for Signer {
    fn from(value: RemoteSigner) -> Self {
        Self::Remote(value)
    }
}

#[derive(Clone, Debug)]
pub struct LocalSigner {
    private_key: SecretKey,
    pub address: Address,
}

impl LocalSigner {
    pub fn new(private_key: SecretKey) -> Self {
        let address = Address::from(keccak(
            &private_key.public_key(SECP256K1).serialize_uncompressed()[1..],
        ));
        Self {
            private_key,
            address,
        }
    }

    pub fn sign(&self, data: Bytes) -> Signature {
        let hash = keccak(data);
        let msg = Message::from_digest(hash.0);
        let (recovery_id, signature) = SECP256K1
            .sign_ecdsa_recoverable(&msg, &self.private_key)
            .serialize_compact();

        Signature::from_slice(&[signature.as_slice(), &[recovery_id.to_i32() as u8]].concat())
    }
}

#[derive(Clone, Debug)]
pub struct RemoteSigner {
    pub url: Url,
    pub public_key: PublicKey,
    pub address: Address,
    pub client: Client,
}

impl RemoteSigner {
    pub fn new(url: Url, public_key: PublicKey) -> Self {
        let address = Address::from(keccak(&public_key.serialize_uncompressed()[1..]));
        Self {
            url,
            public_key,
            address,
            client: Client::new(),
        }
    }

    pub async fn sign(&self, data: Bytes) -> Result<Signature, SignerError> {
        let url = format!(
            "{}api/v1/eth1/sign/{}",
            self.url,
            hex::encode(&self.public_key.serialize_uncompressed()[1..])
        );
        let body = format!("{{\"data\": \"0x{}\"}}", hex::encode(data.clone()));

        self.client
            .post(url)
            .body(body)
            .header("content-type", "application/json")
            .send()
            .await?
            .text()
            .await?
            .parse::<Signature>()
            .map_err(|e| SignerError::ParseError(e.to_string()))
    }
}

fn parse_signature(signature: Signature) -> (U256, U256, bool) {
    let r = U256::from_big_endian(&signature[..32]);
    let s = U256::from_big_endian(&signature[32..64]);
    let y_parity = signature[64] != 0 && signature[64] != 27;

    (r, s, y_parity)
}

#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    #[error("Failed with a reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("Failed to parse value: {0}")]
    ParseError(String),
    #[error("Tried to sign Privileged L2 transaction")]
    PrivilegedL2TxUnsupported,
}
