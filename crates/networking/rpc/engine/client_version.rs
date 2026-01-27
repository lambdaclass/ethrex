use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use crate::{
    rpc::{ClientVersion, RpcApiContext, RpcHandler},
    utils::RpcErr,
};

/// Client version information as defined in the Engine API specification.
///
/// This structure identifies a client implementation with a standardized format
/// that includes both human-readable and machine-readable fields.
///
/// See: https://github.com/ethereum/execution-apis/blob/main/src/engine/identification.md
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientVersionV1 {
    /// Two-letter client code (e.g., "GE" for go-ethereum, "NM" for nethermind).
    /// Ethrex uses "EX".
    pub code: String,
    /// Human-readable name of the client (e.g., "ethrex", "go-ethereum").
    pub name: String,
    /// Version string of the client (e.g., "v0.1.0", "1.0.0-alpha.1").
    pub version: String,
    /// First four bytes of the latest commit hash, hex-encoded (e.g., "fa4ff922").
    pub commit: String,
}

impl ClientVersionV1 {
    /// Creates a new ClientVersionV1 for the ethrex client from the ClientVersion struct.
    pub fn from_client_version(cv: &ClientVersion) -> Self {
        // Take only first 8 characters (4 bytes) of the commit hash
        let commit = if cv.commit.len() >= 8 {
            cv.commit[..8].to_string()
        } else {
            cv.commit.clone()
        };

        Self {
            code: "EX".to_string(),
            name: cv.name.clone(),
            version: format!("v{}", cv.version),
            commit,
        }
    }
}

/// Request handler for `engine_getClientVersionV1`.
///
/// This method allows consensus and execution layer clients to exchange version
/// information. The execution client returns its own version information in response.
#[derive(Debug)]
pub struct GetClientVersionV1Request {
    /// The consensus client's version information (provided as input parameter).
    #[allow(dead_code)]
    consensus_client: ClientVersionV1,
}

impl std::fmt::Display for GetClientVersionV1Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "GetClientVersionV1Request {{ consensus_client: {} {} {} }}",
            self.consensus_client.code, self.consensus_client.name, self.consensus_client.version
        )
    }
}

impl RpcHandler for GetClientVersionV1Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        }
        let consensus_client: ClientVersionV1 = serde_json::from_value(params[0].clone())?;
        Ok(GetClientVersionV1Request { consensus_client })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested engine_getClientVersionV1: {self}");

        // Return an array with a single ClientVersionV1 for this execution client.
        // When connected to multiple execution clients via a multiplexer, the multiplexer
        // would concatenate responses, but ethrex is a single client.
        let client_version =
            ClientVersionV1::from_client_version(&context.node_data.client_version);

        serde_json::to_value(vec![client_version])
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
