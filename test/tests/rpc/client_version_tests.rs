use ethrex_rpc::engine::client_version::{ClientVersionV1, GetClientVersionV1Request};
use ethrex_rpc::rpc::{ClientVersion, RpcHandler};
use ethrex_rpc::test_utils::default_context_with_storage;
use ethrex_rpc::utils::RpcRequest;
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
    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");

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

    // Verify the client version fields match the test context
    // Test context uses: name="ethrex", version="0.1.0", commit="abcd1234"
    let client_version = &response_array[0];
    assert_eq!(client_version["code"], "EX");
    assert_eq!(client_version["name"], "ethrex");
    assert_eq!(client_version["version"], "v0.1.0");
    assert_eq!(client_version["commit"], "abcd1234");
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

    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
    let context = default_context_with_storage(storage).await;
    let result = GetClientVersionV1Request::call(&request, context).await;

    assert!(result.is_ok(), "Should accept unknown client codes");
}

#[test]
fn test_from_client_version() {
    let cv = ClientVersion::new(
        "ethrex".to_string(),
        "0.1.0".to_string(),
        "rpc/engine_getClientVersionV1".to_string(), // Branch with slash
        "abc12345def67890".to_string(),              // Long commit hash
        "x86_64-apple-darwin".to_string(),
        "1.70.0".to_string(),
    );

    let client_version = ClientVersionV1::from_client_version(&cv);

    assert_eq!(client_version.code, "EX");
    assert_eq!(client_version.name, "ethrex");
    assert_eq!(client_version.version, "v0.1.0");
    // Commit should be truncated to 8 characters
    assert_eq!(client_version.commit, "abc12345");
}

#[test]
fn test_from_client_version_full_commit() {
    let cv = ClientVersion::new(
        "ethrex".to_string(),
        "0.1.0".to_string(),
        "main".to_string(),
        "abc12345def67890abc12345def67890abcdef01".to_string(), // Full 40-char SHA
        "x86_64-apple-darwin".to_string(),
        "1.70.0".to_string(),
    );

    let client_version = ClientVersionV1::from_client_version(&cv);

    // Commit should be truncated to first 8 characters (4 bytes)
    assert_eq!(client_version.commit, "abc12345");
}

#[test]
fn test_client_version_v1_serialization() {
    let client_version = ClientVersionV1 {
        code: "EX".to_string(),
        name: "ethrex".to_string(),
        version: "v0.1.0".to_string(),
        commit: "abcd1234".to_string(),
    };

    let serialized = serde_json::to_value(&client_version).unwrap();
    assert_eq!(serialized["code"], "EX");
    assert_eq!(serialized["name"], "ethrex");
    assert_eq!(serialized["version"], "v0.1.0");
    assert_eq!(serialized["commit"], "abcd1234");

    // Test deserialization
    let deserialized: ClientVersionV1 = serde_json::from_value(serialized).unwrap();
    assert_eq!(deserialized, client_version);
}
