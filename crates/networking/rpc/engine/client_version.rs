use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
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
    /// Creates a new ClientVersionV1 for the ethrex client by parsing the client_version string.
    ///
    /// The client_version string has the format:
    /// `{name}/v{version}-{branch}-{commit}/{triple}/rustc-v{rustc_version}`
    ///
    /// For example: `ethrex/v0.1.0-main-abc12345/x86_64-apple-darwin/rustc-v1.70.0`
    pub fn from_client_version_string(client_version: &str) -> Self {
        // Default values in case parsing fails
        let mut name = "ethrex".to_string();
        let mut version = "unknown".to_string();
        let mut commit = "00000000".to_string();

        // Parse the client version string
        // Format: {name}/v{version}-{branch}-{commit}/{triple}/rustc-v{rustc_version}
        let parts: Vec<&str> = client_version.split('/').collect();
        if !parts.is_empty() && !parts[0].is_empty() {
            name = parts[0].to_string();
        }
        if parts.len() > 1 {
            // parts[1] is like "v0.1.0-main-abc12345"
            let version_parts: Vec<&str> = parts[1].split('-').collect();
            if !version_parts.is_empty() {
                version = version_parts[0].to_string();
            }
            // The commit hash is the last part after the last hyphen
            // e.g., "v0.1.0-main-abc12345" -> "abc12345"
            if version_parts.len() >= 3 {
                let sha = version_parts.last().unwrap_or(&"00000000");
                // Take only first 8 characters (4 bytes)
                commit = if sha.len() >= 8 {
                    sha[..8].to_string()
                } else {
                    sha.to_string()
                };
            }
        }

        Self {
            code: "EX".to_string(),
            name,
            version,
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
            ClientVersionV1::from_client_version_string(&context.node_data.client_version);

        serde_json::to_value(vec![client_version])
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::default_context_with_storage;
    use crate::utils::RpcRequest;
    use ethrex_storage::{EngineType, Store};

    #[tokio::test]
    async fn test_get_client_version_v1() {
        // Create request with a mock consensus client version
        let body = r#"{
            "jsonrpc": "2.0",
            "method": "engine_getClientVersionV1",
            "params": [{
                "code": "LH",
                "name": "Lighthouse",
                "version": "v4.6.0",
                "commit": "abcd1234"
            }],
            "id": 1
        }"#;
        let request: RpcRequest = serde_json::from_str(body).unwrap();

        // Setup storage
        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");

        // Process request
        let context = default_context_with_storage(storage).await;
        let result = GetClientVersionV1Request::call(&request, context).await;

        // Verify the response
        assert!(result.is_ok());
        let response_value = result.unwrap();

        // Response should be an array
        let response_array = response_value
            .as_array()
            .expect("Response should be an array");
        assert_eq!(
            response_array.len(),
            1,
            "Should return exactly one client version"
        );

        // Verify the client version fields
        let client_version = &response_array[0];
        assert_eq!(client_version["code"], "EX");
        // The test context uses "ethrex/test" as client_version
        assert_eq!(client_version["name"], "ethrex");
    }

    #[tokio::test]
    async fn test_get_client_version_v1_missing_params() {
        let body = r#"{
            "jsonrpc": "2.0",
            "method": "engine_getClientVersionV1",
            "params": [],
            "id": 1
        }"#;
        let request: RpcRequest = serde_json::from_str(body).unwrap();

        let result = GetClientVersionV1Request::parse(&request.params);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_client_version_v1_no_params() {
        let body = r#"{
            "jsonrpc": "2.0",
            "method": "engine_getClientVersionV1",
            "id": 1
        }"#;
        let request: RpcRequest = serde_json::from_str(body).unwrap();

        let result = GetClientVersionV1Request::parse(&request.params);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_client_version_v1_accepts_unknown_client_codes() {
        // Clients MUST accommodate receiving any two-letter ClientCode
        let body = r#"{
            "jsonrpc": "2.0",
            "method": "engine_getClientVersionV1",
            "params": [{
                "code": "XX",
                "name": "UnknownClient",
                "version": "v1.0.0",
                "commit": "12345678"
            }],
            "id": 1
        }"#;
        let request: RpcRequest = serde_json::from_str(body).unwrap();

        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        let context = default_context_with_storage(storage).await;
        let result = GetClientVersionV1Request::call(&request, context).await;

        assert!(result.is_ok(), "Should accept unknown client codes");
    }

    #[test]
    fn test_from_client_version_string() {
        // Test with a typical version string
        let version_str = "ethrex/v0.1.0-main-abc12345/x86_64-apple-darwin/rustc-v1.70.0";
        let client_version = ClientVersionV1::from_client_version_string(version_str);

        assert_eq!(client_version.code, "EX");
        assert_eq!(client_version.name, "ethrex");
        assert_eq!(client_version.version, "v0.1.0");
        assert_eq!(client_version.commit, "abc12345");
    }
}
