use bytes::Bytes;
use ethereum_types::{Address, Signature};
use ethrex_common::{
    U256,
    types::{
        EIP1559Transaction, EIP2930Transaction, EIP4844Transaction, EIP7702Transaction,
        LegacyTransaction, Transaction, TxType,
    },
};
use ethrex_rlp::encode::PayloadRLPEncode;
use keccak_hash::keccak;
use reqwest::{Client, StatusCode, Url};
use rustc_hex::FromHexError;
use secp256k1::{Message, PublicKey, SECP256K1, SecretKey};
use url::ParseError;

#[derive(Clone, Debug)]
pub enum Signer {
    Local(LocalSigner),
    Remote(RemoteSigner),
}

impl Signer {
    pub async fn sign(&self, data: Bytes) -> Result<Signature, SignerError> {
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
    pub private_key: SecretKey,
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
}

impl RemoteSigner {
    pub fn new(url: Url, public_key: PublicKey) -> Self {
        let address = Address::from(keccak(&public_key.serialize_uncompressed()[1..]));
        Self {
            url,
            public_key,
            address,
        }
    }

    pub async fn sign(&self, data: Bytes) -> Result<Signature, SignerError> {
        let url = self
            .url
            .join("api/v1/eth1/sign/")?
            .join(&hex::encode(&self.public_key.serialize_uncompressed()[1..]))?;
        let body = format!("{{\"data\": \"0x{}\"}}", hex::encode(data));

        let client = Client::new();
        let response = client
            .post(url)
            .body(body)
            .header("content-type", "application/json")
            .send()
            .await?;

        match response.status() {
            StatusCode::OK => response
                .text()
                .await?
                .parse::<Signature>()
                .map_err(SignerError::FromHexError),
            StatusCode::NOT_FOUND => Err(SignerError::Web3SignerError(
                "Private key not found in web3signer server".to_string(),
            )),
            StatusCode::BAD_REQUEST => Err(SignerError::Web3SignerError(
                "Bad request format".to_string(),
            )),
            StatusCode::INTERNAL_SERVER_ERROR => Err(SignerError::Web3SignerError(
                "Internal server error".to_string(),
            )),
            _ => Err(SignerError::Web3SignerError(format!(
                "Unknown error {}",
                response.status().as_str(),
            ))),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    #[error("Url Parse Error: {0}")]
    ParseError(#[from] ParseError),
    #[error("Failed with a reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("Failed to parse value: {0}")]
    FromHexError(#[from] FromHexError),
    #[error("Tried to sign Privileged L2 transaction")]
    PrivilegedL2TxUnsupported,
    #[error("Web3signer error: {0}")]
    Web3SignerError(String),
}

fn parse_signature(signature: Signature) -> (U256, U256, bool) {
    let r = U256::from_big_endian(&signature[..32]);
    let s = U256::from_big_endian(&signature[32..64]);
    let y_parity = signature[64] != 0 && signature[64] != 27;

    (r, s, y_parity)
}

pub trait Signable {
    fn sign(
        &self,
        signer: &Signer,
    ) -> impl std::future::Future<Output = Result<Self, SignerError>> + Send
    where
        Self: Sized + Sync + Send + Clone,
    {
        async {
            let mut signable = self.clone();
            signable.sign_inplace(signer).await?;
            Ok(signable)
        }
    }

    fn sign_inplace(
        &mut self,
        signer: &Signer,
    ) -> impl std::future::Future<Output = Result<(), SignerError>> + Send;
}

impl Signable for Transaction {
    async fn sign_inplace(&mut self, signer: &Signer) -> Result<(), SignerError> {
        match self {
            Transaction::LegacyTransaction(tx) => tx.sign_inplace(signer).await,
            Transaction::EIP2930Transaction(tx) => tx.sign_inplace(signer).await,
            Transaction::EIP1559Transaction(tx) => tx.sign_inplace(signer).await,
            Transaction::EIP4844Transaction(tx) => tx.sign_inplace(signer).await,
            Transaction::EIP7702Transaction(tx) => tx.sign_inplace(signer).await,
            Transaction::PrivilegedL2Transaction(_) => Err(SignerError::PrivilegedL2TxUnsupported), // Privileged Transactions are not signed
        }
    }
}

impl Signable for LegacyTransaction {
    async fn sign_inplace(&mut self, signer: &Signer) -> Result<(), SignerError> {
        let signature = signer.sign(self.encode_payload_to_vec().into()).await?;

        self.v = U256::from(signature[64]);
        (self.r, self.s, _) = parse_signature(signature);

        Ok(())
    }
}

impl Signable for EIP1559Transaction {
    async fn sign_inplace(&mut self, signer: &Signer) -> Result<(), SignerError> {
        let mut payload = vec![TxType::EIP1559 as u8];
        payload.append(self.encode_payload_to_vec().as_mut());

        let signature = signer.sign(payload.into()).await?;
        (self.signature_r, self.signature_s, self.signature_y_parity) = parse_signature(signature);

        Ok(())
    }
}

impl Signable for EIP2930Transaction {
    async fn sign_inplace(&mut self, signer: &Signer) -> Result<(), SignerError> {
        let mut payload = vec![TxType::EIP2930 as u8];
        payload.append(self.encode_payload_to_vec().as_mut());

        let signature = signer.sign(payload.into()).await?;
        (self.signature_r, self.signature_s, self.signature_y_parity) = parse_signature(signature);

        Ok(())
    }
}

impl Signable for EIP4844Transaction {
    async fn sign_inplace(&mut self, signer: &Signer) -> Result<(), SignerError> {
        let mut payload = vec![TxType::EIP4844 as u8];
        payload.append(self.encode_payload_to_vec().as_mut());

        let signature = signer.sign(payload.into()).await?;
        (self.signature_r, self.signature_s, self.signature_y_parity) = parse_signature(signature);

        Ok(())
    }
}

impl Signable for EIP7702Transaction {
    async fn sign_inplace(&mut self, signer: &Signer) -> Result<(), SignerError> {
        let mut payload = vec![TxType::EIP7702 as u8];
        payload.append(self.encode_payload_to_vec().as_mut());

        let signature = signer.sign(payload.into()).await?;
        (self.signature_r, self.signature_s, self.signature_y_parity) = parse_signature(signature);

        Ok(())
    }
}
