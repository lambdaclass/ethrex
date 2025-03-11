use errors::BeaconClientError;
use ethrex_common::{H256, U256};
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::Value;
use types::{BlobSidecar, GetBlockResponseData};

pub mod errors;
pub mod types;

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum BeaconResponse {
    Success(BeaconResponseSuccess),
    Error(BeaconResponseError),
}

#[derive(Deserialize, Debug)]
pub struct BeaconResponseSuccess {
    data: Value,
}

#[derive(Deserialize, Debug)]
pub struct BeaconResponseError {
    code: u64,
    message: String,
}

pub struct BeaconClient {
    client: Client,
    url: Url,
}

impl BeaconClient {
    pub fn new(url: Url) -> Self {
        Self {
            client: Client::new(),
            url,
        }
    }

    async fn send_request<T>(&self, endpoint: &str) -> Result<T, BeaconClientError>
    where
        T: serde::de::DeserializeOwned,
    {
        println!("Sending request: {endpoint}");
        let response = self
            .client
            .get(self.url.clone().join(endpoint).unwrap())
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .send()
            .await?
            .json::<BeaconResponse>()
            .await
            .map_err(BeaconClientError::from)?;

        match response {
            BeaconResponse::Success(res) => serde_json::from_value::<T>(res.data)
                .map_err(|_| BeaconClientError::Custom("Unexpected format".to_string())),
            BeaconResponse::Error(err) => Err(BeaconClientError::Custom(err.message)),
        }
    }

    pub async fn get_block_by_hash(
        &self,
        block_hash: H256,
    ) -> Result<GetBlockResponseData, BeaconClientError> {
        self.send_request(&format!("/eth/v2/beacon/blocks/{block_hash:#x}"))
            .await
    }

    pub async fn get_blobs_by_slot(
        &self,
        slot: U256,
    ) -> Result<Vec<BlobSidecar>, BeaconClientError> {
        self.send_request(&format!("/eth/v1/beacon/blob_sidecars/{slot}"))
            .await
    }
}
