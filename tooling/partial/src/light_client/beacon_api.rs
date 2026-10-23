//! The beacon REST API as the light client reads it. It can stall the light client but
//! cannot make it accept a wrong header: every object is verified by [`super::sync`].

use std::collections::BTreeMap;
use std::time::Duration;

use ethrex_common::H256;
use reqwest::{Client, Url};
use serde::de::DeserializeOwned;

use super::sync::LightClientSpec;
use super::types::{
    GenesisResponse, HeaderData, HeaderResponse, LightClientBootstrap, LightClientUpdate, Versioned,
};

/// Most sync committee periods one `updates` request asks for.
pub const MAX_UPDATES_PER_REQUEST: u64 = 128;

/// `FAR_FUTURE_EPOCH`: a fork not scheduled.
const FAR_FUTURE_EPOCH: u64 = u64::MAX;

/// A beacon API request failed.
#[derive(Debug, thiserror::Error)]
pub enum BeaconApiError {
    /// The URL is malformed.
    #[error("bad beacon API url: {0}")]
    Url(String),
    /// Transport failure.
    #[error("beacon API request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// The server answered with an error status.
    #[error("beacon API {path} answered {status}")]
    Status {
        /// Path requested.
        path: String,
        /// HTTP status.
        status: u16,
    },
    /// The body did not parse.
    #[error("beacon API {path} returned an unexpected body: {reason}")]
    Body {
        /// Path requested.
        path: String,
        /// Why.
        reason: String,
    },
    /// The spec lacks or mangles a value the light client needs.
    #[error("beacon API spec is missing {0}")]
    Spec(String),
}

/// A beacon REST API client.
#[derive(Clone, Debug)]
pub struct BeaconApi {
    client: Client,
    base: Url,
}

impl BeaconApi {
    /// A client for the beacon node at `base`. A path prefix in `base` (as gateways that
    /// front several nodes use) is kept in front of every request path.
    pub fn new(base: Url, timeout: Duration) -> Result<Self, BeaconApiError> {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(BeaconApiError::Http)?;
        let mut base = base;
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        Ok(Self { client, base })
    }

    /// The full URL of an API path.
    pub fn url(&self, path: &str) -> Result<Url, BeaconApiError> {
        self.base
            .join(path.trim_start_matches('/'))
            .map_err(|e| BeaconApiError::Url(e.to_string()))
    }

    /// The base URL.
    pub fn base(&self) -> &Url {
        &self.base
    }

    pub(crate) async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, BeaconApiError> {
        let url = self.url(path)?;
        let response = self
            .client
            .get(url)
            .header("accept", "application/json")
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(BeaconApiError::Status {
                path: path.to_string(),
                status: status.as_u16(),
            });
        }
        let body = response.bytes().await?;
        serde_json::from_slice(&body).map_err(|e| BeaconApiError::Body {
            path: path.to_string(),
            reason: e.to_string(),
        })
    }

    /// `GET /eth/v1/config/spec`, as a map of strings.
    pub async fn spec(&self) -> Result<BTreeMap<String, String>, BeaconApiError> {
        #[derive(serde::Deserialize)]
        struct SpecResponse {
            data: BTreeMap<String, serde_json::Value>,
        }
        let response: SpecResponse = self.get_json("/eth/v1/config/spec").await?;
        Ok(response
            .data
            .into_iter()
            .map(|(k, v)| {
                let s = match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                };
                (k, s)
            })
            .collect())
    }

    /// `GET /eth/v1/beacon/genesis`.
    pub async fn genesis(&self) -> Result<GenesisResponse, BeaconApiError> {
        self.get_json("/eth/v1/beacon/genesis").await
    }

    /// The light client's constants, read from the spec and genesis.
    pub async fn light_client_spec(&self) -> Result<LightClientSpec, BeaconApiError> {
        let spec = self.spec().await?;
        let genesis = self.genesis().await?.data;
        light_client_spec_from(
            &spec,
            genesis.genesis_time,
            H256(genesis.genesis_validators_root),
            genesis.genesis_fork_version,
        )
    }

    /// `GET /eth/v1/beacon/light_client/bootstrap/{root}`.
    pub async fn bootstrap(&self, root: H256) -> Result<LightClientBootstrap, BeaconApiError> {
        let response: Versioned<LightClientBootstrap> = self
            .get_json(&format!("/eth/v1/beacon/light_client/bootstrap/{root:#x}"))
            .await?;
        Ok(response.data)
    }

    /// `GET /eth/v1/beacon/light_client/updates?start_period=..&count=..`.
    pub async fn updates(
        &self,
        start_period: u64,
        count: u64,
    ) -> Result<Vec<LightClientUpdate>, BeaconApiError> {
        let count = count.clamp(1, MAX_UPDATES_PER_REQUEST);
        let response: Vec<Versioned<LightClientUpdate>> = self
            .get_json(&format!(
                "/eth/v1/beacon/light_client/updates?start_period={start_period}&count={count}"
            ))
            .await?;
        Ok(response.into_iter().map(|u| u.data).collect())
    }

    /// `GET /eth/v1/beacon/light_client/finality_update`.
    pub async fn finality_update(&self) -> Result<LightClientUpdate, BeaconApiError> {
        let response: Versioned<LightClientUpdate> = self
            .get_json("/eth/v1/beacon/light_client/finality_update")
            .await?;
        Ok(response.data)
    }

    /// `GET /eth/v1/beacon/light_client/optimistic_update`.
    pub async fn optimistic_update(&self) -> Result<LightClientUpdate, BeaconApiError> {
        let response: Versioned<LightClientUpdate> = self
            .get_json("/eth/v1/beacon/light_client/optimistic_update")
            .await?;
        Ok(response.data)
    }

    /// `GET /eth/v1/beacon/headers/{id}`: a header by root, slot or tag. The caller checks
    /// that the header hashes to the root it expects.
    pub async fn header(&self, id: &str) -> Result<HeaderData, BeaconApiError> {
        let response: HeaderResponse = self
            .get_json(&format!("/eth/v1/beacon/headers/{id}"))
            .await?;
        Ok(response.data)
    }
}

/// Builds the light client spec from the `/eth/v1/config/spec` map.
pub fn light_client_spec_from(
    spec: &BTreeMap<String, String>,
    genesis_time: u64,
    genesis_validators_root: H256,
    genesis_fork_version: [u8; 4],
) -> Result<LightClientSpec, BeaconApiError> {
    let epoch = |name: &str| -> Result<Option<u64>, BeaconApiError> {
        match spec.get(name) {
            None => Ok(None),
            Some(value) => {
                let epoch: u64 = value
                    .parse()
                    .map_err(|_| BeaconApiError::Spec(format!("{name}={value}")))?;
                Ok((epoch != FAR_FUTURE_EPOCH).then_some(epoch))
            }
        }
    };
    let version = |name: &str| -> Result<[u8; 4], BeaconApiError> {
        let value = spec
            .get(name)
            .ok_or_else(|| BeaconApiError::Spec(name.to_string()))?;
        let bytes = hex::decode(value.trim_start_matches("0x"))
            .map_err(|_| BeaconApiError::Spec(format!("{name}={value}")))?;
        <[u8; 4]>::try_from(bytes.as_slice())
            .map_err(|_| BeaconApiError::Spec(format!("{name}={value}")))
    };
    let mut forks = vec![(0u64, genesis_fork_version)];
    for (key, _) in spec.iter() {
        let Some(fork) = key.strip_suffix("_FORK_EPOCH") else {
            continue;
        };
        let Some(activation) = epoch(key)? else {
            continue;
        };
        let fork_version = version(&format!("{fork}_FORK_VERSION"))?;
        forks.push((activation, fork_version));
    }
    forks.sort();
    forks.dedup();
    let seconds_per_slot = spec
        .get("SECONDS_PER_SLOT")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| BeaconApiError::Spec("SECONDS_PER_SLOT".into()))?;
    let min_sync_committee_participants = spec
        .get("MIN_SYNC_COMMITTEE_PARTICIPANTS")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    Ok(LightClientSpec {
        genesis_validators_root,
        genesis_time,
        seconds_per_slot,
        forks,
        capella_fork_epoch: epoch("CAPELLA_FORK_EPOCH")?,
        deneb_fork_epoch: epoch("DENEB_FORK_EPOCH")?,
        electra_fork_epoch: epoch("ELECTRA_FORK_EPOCH")?,
        gloas_fork_epoch: epoch("GLOAS_FORK_EPOCH")?,
        min_sync_committee_participants,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_path_prefix_in_the_base_url_is_kept() {
        let api = BeaconApi::new(
            "https://beacon.example.org/nimbus".parse().unwrap(),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(
            api.url("/eth/v1/beacon/genesis").unwrap().as_str(),
            "https://beacon.example.org/nimbus/eth/v1/beacon/genesis"
        );
        let api = BeaconApi::new(
            "http://localhost:5052".parse().unwrap(),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(
            api.url("eth/v1/config/spec").unwrap().as_str(),
            "http://localhost:5052/eth/v1/config/spec"
        );
    }

    #[test]
    fn spec_file_builds_the_sepolia_constants() {
        #[derive(serde::Deserialize)]
        struct SpecFile {
            data: BTreeMap<String, String>,
        }
        let path = format!("{}/testdata/sepolia/spec.json", env!("CARGO_MANIFEST_DIR"));
        let spec: SpecFile = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let genesis_path = format!(
            "{}/testdata/sepolia/genesis.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let genesis: GenesisResponse =
            serde_json::from_str(&std::fs::read_to_string(genesis_path).unwrap()).unwrap();
        let built = light_client_spec_from(
            &spec.data,
            genesis.data.genesis_time,
            H256(genesis.data.genesis_validators_root),
            genesis.data.genesis_fork_version,
        )
        .unwrap();
        assert_eq!(built, crate::light_client::sync::tests::sepolia());
    }
}
