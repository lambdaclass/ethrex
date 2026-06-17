//! Integration tests for the engine REST/SSZ sub-router, helpers, and
//! wire types (migrated from crates/networking/rpc/engine_rest/tests.rs).

mod test_helpers {
    /// Build a valid JWT bearer token for the given secret, with `iat` set to now.
    pub async fn auth_token(secret: &[u8]) -> String {
        use jsonwebtoken::{EncodingKey, Header, encode};
        #[derive(serde::Serialize)]
        struct C {
            iat: u64,
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        encode(
            &Header::default(),
            &C { iat: now },
            &EncodingKey::from_secret(secret),
        )
        .unwrap()
    }
}
mod problem_json_tests {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use ethrex_rpc::engine_rest::error::ProblemJson;
    use http_body_util::BodyExt;

    #[tokio::test]
    async fn problem_json_serializes_with_correct_content_type_and_status() {
        let problem =
            ProblemJson::not_implemented("Endpoint registered but handler pending sub-project 2/3");
        let response = problem.into_response();

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
        let ct = response
            .headers()
            .get("content-type")
            .expect("missing content-type")
            .to_str()
            .unwrap();
        assert_eq!(ct, "application/problem+json");

        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(v["type"], "about:blank");
        assert_eq!(v["title"], "Not Implemented");
        assert_eq!(v["status"], 501);
        assert_eq!(
            v["detail"],
            "Endpoint registered but handler pending sub-project 2/3"
        );
    }

    #[tokio::test]
    async fn problem_json_bad_request_helper_sets_400() {
        let problem = ProblemJson::bad_request("unsupported fork: frontier");
        let response = problem.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn problem_json_omits_optional_fields_when_none() {
        let problem = ProblemJson {
            typ: "about:blank".into(),
            title: "Test".into(),
            status: 400,
            detail: None,
            instance: None,
        };
        let response = problem.into_response();
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let s = String::from_utf8(body_bytes.to_vec()).unwrap();
        assert!(!s.contains("detail"));
        assert!(!s.contains("instance"));
    }
}

mod fork_path_tests {
    use ethrex_common::types::Fork;
    use ethrex_rpc::engine_rest::fork_path::parse_fork_segment;

    #[test]
    fn parse_supported_forks() {
        assert_eq!(parse_fork_segment("paris").unwrap(), Fork::Paris);
        assert_eq!(parse_fork_segment("shanghai").unwrap(), Fork::Shanghai);
        assert_eq!(parse_fork_segment("cancun").unwrap(), Fork::Cancun);
        assert_eq!(parse_fork_segment("prague").unwrap(), Fork::Prague);
        assert_eq!(parse_fork_segment("osaka").unwrap(), Fork::Osaka);
        assert_eq!(parse_fork_segment("amsterdam").unwrap(), Fork::Amsterdam);
    }

    #[test]
    fn rejects_historical_forks_not_in_engine_spec() {
        assert!(parse_fork_segment("frontier").is_err());
        assert!(parse_fork_segment("homestead").is_err());
        assert!(parse_fork_segment("london").is_err());
    }

    #[test]
    fn rejects_unknown_strings() {
        assert!(parse_fork_segment("").is_err());
        assert!(parse_fork_segment("PARIS").is_err()); // case-sensitive
        assert!(parse_fork_segment("not-a-fork").is_err());
    }

    #[tokio::test]
    async fn extractor_rejects_unknown_fork_with_400() {
        use axum::Router;
        use axum::routing::post;
        use ethrex_rpc::engine_rest::fork_path::ForkPath;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        async fn handler(ForkPath(_fork): ForkPath) -> &'static str {
            "ok"
        }

        let app: Router<()> = Router::new().route("/{fork}/payloads", post(handler));

        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/frontier/payloads")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(request).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["status"], 400);
    }
}

mod auth_tests {
    use axum::Router;
    use axum::http::StatusCode;
    use axum::routing::get;
    use bytes::Bytes;
    use ethrex_rpc::engine_rest::auth::engine_auth_middleware;
    use http_body_util::BodyExt;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde::Serialize;
    use tower::ServiceExt;

    #[derive(Serialize)]
    struct Claims {
        iat: u64,
    }

    fn make_jwt(secret: &[u8], iat_offset_secs: i64) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let iat = (now + iat_offset_secs).max(0) as u64;
        encode(
            &Header::default(),
            &Claims { iat },
            &EncodingKey::from_secret(secret),
        )
        .unwrap()
    }

    fn test_app(secret: Bytes) -> Router<()> {
        Router::new()
            .route("/protected", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                secret,
                engine_auth_middleware,
            ))
    }

    #[tokio::test]
    async fn rejects_missing_authorization_header() {
        let secret = Bytes::from(vec![0xAB; 32]);
        let app = test_app(secret);
        let req = axum::http::Request::builder()
            .uri("/protected")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
    }

    #[tokio::test]
    async fn rejects_wrong_secret() {
        let app_secret = Bytes::from(vec![0xAB; 32]);
        let signing_secret = vec![0xCD; 32];
        let token = make_jwt(&signing_secret, 0);
        let app = test_app(app_secret);
        let req = axum::http::Request::builder()
            .uri("/protected")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn accepts_valid_jwt_and_passes_through() {
        let secret = vec![0xAB; 32];
        let token = make_jwt(&secret, 0);
        let app = test_app(Bytes::from(secret));
        let req = axum::http::Request::builder()
            .uri("/protected")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn captures_client_version_header_into_extensions() {
        use ethrex_rpc::engine_rest::auth::EngineClientVersion;

        let secret = vec![0xAB; 32];
        let token = make_jwt(&secret, 0);

        async fn handler(req: axum::http::Request<axum::body::Body>) -> String {
            req.extensions()
                .get::<EngineClientVersion>()
                .map(|cv| cv.raw.clone())
                .unwrap_or_else(|| "missing".to_string())
        }

        let app: Router<()> = Router::new().route("/protected", get(handler)).layer(
            axum::middleware::from_fn_with_state(Bytes::from(secret), engine_auth_middleware),
        );
        let req = axum::http::Request::builder()
            .uri("/protected")
            .header("authorization", format!("Bearer {token}"))
            .header("x-engine-client-version", "lighthouse/v5.0.0/abcd1234/rust")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"lighthouse/v5.0.0/abcd1234/rust");
    }
}

// stub_tests removed: all stubs replaced by real handlers in sub-project 3.

mod identity_tests {
    use axum::Router;
    use axum::routing::get;
    use ethrex_rpc::engine::client_version::ClientVersionV1;
    use ethrex_rpc::engine_rest::handlers::identity::get_identity;
    use ethrex_rpc::rpc::ClientVersion;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn make_client_version() -> ClientVersion {
        ClientVersion::new(
            "ethrex".to_string(),
            "0.1.0".to_string(),
            "test".to_string(),
            "abcd1234ef".to_string(),
            "x86_64-unknown-linux".to_string(),
            "1.85.0".to_string(),
        )
    }

    #[tokio::test]
    async fn returns_client_version_array_as_json() {
        let cv = make_client_version();
        let app: Router<()> = Router::new()
            .route("/identity", get(get_identity))
            .with_state(cv.clone());

        let req = axum::http::Request::builder()
            .uri("/identity")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let arr: Vec<ClientVersionV1> = serde_json::from_slice(&body).unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].code, "EX");
        assert_eq!(arr[0].name, "ethrex");
        assert_eq!(arr[0].version, "v0.1.0");
        assert_eq!(arr[0].commit, "abcd1234");
    }
}

mod capabilities_tests {
    use axum::Router;
    use axum::routing::get;
    use ethrex_rpc::engine_rest::handlers::capabilities::{
        BLOBS_MAX_COUNT, BODIES_MAX_COUNT, Capabilities, PAYLOAD_MAX_BYTES, get_capabilities,
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn returns_expected_shape() {
        let app: Router<()> = Router::new().route("/capabilities", get(get_capabilities));
        let req = axum::http::Request::builder()
            .uri("/capabilities")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let caps: Capabilities = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            caps.supported_forks,
            vec![
                "paris",
                "shanghai",
                "cancun",
                "prague",
                "osaka",
                "amsterdam"
            ]
        );
        assert_eq!(
            caps.independently_versioned.blobs,
            vec!["v1", "v2", "v3", "v4"]
        );
        assert_eq!(
            caps.fork_scoped_endpoints,
            vec!["payloads", "forkchoice", "bodies"]
        );
        assert_eq!(caps.unscoped_endpoints, vec!["capabilities", "identity"]);
        // Flat dot-notation limit keys with scalar values per #793 refactor.md,
        // not method+path keys with nested objects.
        assert_eq!(caps.limits["payload.max_bytes"], PAYLOAD_MAX_BYTES);
        assert_eq!(caps.limits["bodies.max_count"], BODIES_MAX_COUNT as u64);
        assert_eq!(
            caps.limits["blobs.max_versioned_hashes"],
            BLOBS_MAX_COUNT as u64
        );
    }
}

mod router_tests {
    use axum::http::StatusCode;
    use ethrex_rpc::engine_rest::router;
    use ethrex_rpc::test_utils::default_context_with_storage;
    use ethrex_rpc::test_utils::setup_store;
    use http_body_util::BodyExt;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde::Serialize;
    use tower::ServiceExt;

    #[derive(Serialize)]
    struct Claims {
        iat: u64,
    }

    fn make_jwt(secret: &[u8]) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        encode(
            &Header::default(),
            &Claims { iat: now },
            &EncodingKey::from_secret(secret),
        )
        .unwrap()
    }

    async fn build_app() -> (axum::Router, bytes::Bytes) {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = bytes::Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        (app, secret)
    }

    #[tokio::test]
    async fn identity_returns_array() {
        let (app, secret) = build_app().await;
        let token = make_jwt(&secret);
        let req = axum::http::Request::builder()
            .uri("/identity")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let arr: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(arr.is_array());
        assert_eq!(arr[0]["code"], "EX");
    }

    #[tokio::test]
    async fn capabilities_returns_object() {
        let (app, secret) = build_app().await;
        let token = make_jwt(&secret);
        let req = axum::http::Request::builder()
            .uri("/capabilities")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(v["supported_forks"].is_array());
        assert!(v["fork_scoped_endpoints"].is_array());
        assert!(v["independently_versioned"]["blobs"].is_array());
        assert!(v["unscoped_endpoints"].is_array());
        assert!(v["limits"].is_object());
        assert!(v["limits"]["payload.max_bytes"].is_number());
    }

    #[tokio::test]
    async fn payloads_real_handler_rejects_missing_content_type() {
        // The real submit_payload handler now requires content-type: application/octet-stream.
        // A request with no content-type gets 415.
        let (app, secret) = build_app().await;
        let token = make_jwt(&secret);
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/cancun/payloads")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn missing_auth_returns_401_for_all_routes() {
        let (app, _) = build_app().await;
        for (method, uri) in [
            ("GET", "/identity"),
            ("GET", "/capabilities"),
            ("POST", "/cancun/payloads"),
            ("POST", "/blobs/v1"),
        ] {
            let app = app.clone();
            let req = axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap();
            let resp = app.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{method} {uri}");
        }
    }
}

mod coexistence_tests {
    use axum::Router;
    use axum::http::StatusCode;
    use axum::routing::post;
    use ethrex_rpc::engine_rest::router as engine_rest_router;
    use ethrex_rpc::test_utils::default_context_with_storage;
    use ethrex_rpc::test_utils::setup_store;
    use tower::ServiceExt;

    /// Stand-in for the authrpc JSON-RPC handler — just confirms POST / is reachable.
    async fn fake_jsonrpc_handler() -> &'static str {
        "{\"jsonrpc\":\"2.0\",\"result\":null,\"id\":1}"
    }

    #[tokio::test]
    async fn jsonrpc_and_engine_rest_coexist_on_same_router() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = bytes::Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();

        // Merge in the same way rpc.rs will after this task.
        let authrpc: Router = Router::new()
            .route("/", post(fake_jsonrpc_handler))
            .merge(engine_rest_router(ctx));

        // JSON-RPC POST / works without engine REST's auth (existing flow handles it).
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = authrpc.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        // /identity requires engine REST auth, and gets it.
        use jsonwebtoken::{EncodingKey, Header, encode};
        #[derive(serde::Serialize)]
        struct Claims {
            iat: u64,
        }
        let iat = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = encode(
            &Header::default(),
            &Claims { iat },
            &EncodingKey::from_secret(&secret),
        )
        .unwrap();
        let req = axum::http::Request::builder()
            .uri("/identity")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = authrpc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn engine_rest_does_not_leak_auth_to_root_jsonrpc() {
        // Even without an Authorization header, POST / (JSON-RPC) must still hit
        // its own handler. Engine REST auth must only apply to the engine_rest sub-router.
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        ctx.node_data.jwt_secret = bytes::Bytes::from(vec![0xAB; 32]);

        let authrpc: Router = Router::new()
            .route("/", post(fake_jsonrpc_handler))
            .merge(engine_rest_router(ctx));

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = authrpc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "JSON-RPC must not get 401");
    }
}

mod wire_tests {
    use axum::Router;
    use axum::extract::DefaultBodyLimit;
    use axum::http::StatusCode;
    use axum::routing::post;
    use ethrex_rpc::engine_rest::extractors::Ssz;
    use ethrex_rpc::engine_rest::responses::SszBody;
    use http_body_util::BodyExt;
    use libssz::{SszDecode, SszEncode};
    use libssz_derive::{SszDecode, SszEncode};
    use tower::ServiceExt;

    #[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
    struct TestMsg {
        a: u32,
        b: u64,
    }

    async fn echo(Ssz(msg): Ssz<TestMsg>) -> SszBody<TestMsg> {
        SszBody(msg)
    }

    fn echo_app() -> Router<()> {
        Router::new()
            .route("/echo", post(echo))
            .layer(DefaultBodyLimit::max(1024 * 1024))
    }

    #[tokio::test]
    async fn round_trips_valid_ssz_payload() {
        let msg = TestMsg {
            a: 0xDEADBEEF,
            b: 0xCAFEBABE_0F00BAA1,
        };
        let bytes = msg.to_ssz();

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/echo")
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(bytes.clone()))
            .unwrap();
        let resp = echo_app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/octet-stream"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let decoded = TestMsg::from_ssz_bytes(&body).unwrap();
        assert_eq!(decoded, msg);
    }

    #[tokio::test]
    async fn rejects_wrong_content_type_with_415() {
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/echo")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(vec![0u8; 12]))
            .unwrap();
        let resp = echo_app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
    }

    #[tokio::test]
    async fn rejects_malformed_ssz_with_400() {
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/echo")
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(vec![0u8; 5])) // wrong length
            .unwrap();
        let resp = echo_app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
    }

    #[tokio::test]
    async fn rejects_missing_content_type_with_415() {
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/echo")
            .body(axum::body::Body::from(vec![0u8; 12]))
            .unwrap();
        let resp = echo_app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn rejects_oversize_body_with_413() {
        let app: axum::Router<()> = axum::Router::new()
            .route("/echo", axum::routing::post(echo))
            .layer(axum::extract::DefaultBodyLimit::max(4));
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/echo")
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(vec![0u8; 12]))
            .unwrap();
        let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
    }
}

mod common_types_tests {
    use ethrex_rpc::engine_rest::types::common::{
        ForkchoiceResponse, ForkchoiceState, PayloadId, PayloadStatus, PayloadStatusCode,
    };
    use libssz::{SszDecode, SszEncode};

    #[test]
    fn payload_status_roundtrips_valid() {
        let s = PayloadStatus::new(PayloadStatusCode::Valid as u8, Some([0xAB; 32]), None);
        let bytes = s.to_ssz();
        let back = PayloadStatus::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back.status, 0);
        assert_eq!(back.latest_valid_hash(), Some([0xAB; 32]));
        assert!(back.validation_error_string().is_none());
    }

    #[test]
    fn payload_status_roundtrips_invalid_with_message() {
        let s = PayloadStatus::new(
            PayloadStatusCode::Invalid as u8,
            None,
            Some("bad parent".to_string()),
        );
        let bytes = s.to_ssz();
        let back = PayloadStatus::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back.status, 1);
        assert!(back.latest_valid_hash().is_none());
        assert_eq!(
            back.validation_error_string().as_deref(),
            Some("bad parent")
        );
    }

    #[test]
    fn payload_status_code_values_match_spec() {
        assert_eq!(PayloadStatusCode::Valid as u8, 0);
        assert_eq!(PayloadStatusCode::Invalid as u8, 1);
        assert_eq!(PayloadStatusCode::Syncing as u8, 2);
        assert_eq!(PayloadStatusCode::Accepted as u8, 3);
    }

    #[test]
    fn forkchoice_state_roundtrips() {
        let s = ForkchoiceState {
            head_block_hash: [1; 32],
            safe_block_hash: [2; 32],
            finalized_block_hash: [3; 32],
        };
        let bytes = s.to_ssz();
        let back = ForkchoiceState::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back.head_block_hash, [1; 32]);
        assert_eq!(back.safe_block_hash, [2; 32]);
        assert_eq!(back.finalized_block_hash, [3; 32]);
    }

    #[test]
    fn payload_id_roundtrips() {
        let id = PayloadId([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        let bytes = id.to_ssz();
        assert_eq!(bytes.len(), 8);
        let back = PayloadId::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back.0, id.0);
    }

    #[test]
    fn payload_id_hex_parse_round_trip() {
        let raw = "0x0102030405060708";
        let id: PayloadId = raw.parse().unwrap();
        assert_eq!(id.0, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        assert_eq!(id.to_hex_string(), "0x0102030405060708");
    }

    #[test]
    fn payload_id_hex_parse_rejects_invalid() {
        assert!("01020304".parse::<PayloadId>().is_err()); // missing 0x
        assert!("0x010203".parse::<PayloadId>().is_err()); // too short
        assert!("0xZZ02030405060708".parse::<PayloadId>().is_err()); // bad hex
        assert!("0x01020304050607080900".parse::<PayloadId>().is_err()); // too long
    }

    #[test]
    fn payload_status_truncates_oversize_validation_error() {
        // `PayloadStatus::new` caps `validation_error` at MAX_ERROR_BYTES (1024)
        // so it always fits the `List[byte, 1024]` bound; a 1025-byte input is
        // truncated and round-trips cleanly.
        let huge = "x".repeat(1025);
        let s = PayloadStatus::new(1, None, Some(huge));
        let bytes = s.to_ssz();
        let back = PayloadStatus::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back.validation_error_string().unwrap().len(), 1024);
    }

    #[test]
    fn forkchoice_response_roundtrips_with_payload_id() {
        let r = ForkchoiceResponse::new(
            PayloadStatus::new(PayloadStatusCode::Valid as u8, Some([0xAA; 32]), None),
            Some(PayloadId([1, 2, 3, 4, 5, 6, 7, 8])),
        );
        let bytes = r.to_ssz();
        let back = ForkchoiceResponse::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn forkchoice_response_roundtrips_without_payload_id() {
        let r = ForkchoiceResponse::new(
            PayloadStatus::new(PayloadStatusCode::Syncing as u8, None, None),
            None,
        );
        let bytes = r.to_ssz();
        let back = ForkchoiceResponse::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, r);
        assert!(back.payload_id().is_none());
    }
}

mod paris_types_tests {
    use ethrex_rpc::engine_rest::types::paris::{
        Bytes20, ExecutionPayload as ParisPayload, ExecutionPayloadEnvelope as ParisEnvelope,
        PayloadAttributes as ParisAttrs,
    };
    use libssz::{SszDecode, SszEncode};

    fn sample_payload() -> ParisPayload {
        ParisPayload {
            parent_hash: [1; 32],
            fee_recipient: Bytes20([2; 20]),
            state_root: [3; 32],
            receipts_root: [4; 32],
            logs_bloom: vec![5u8; 256].try_into().expect("logs_bloom length"),
            prev_randao: [6; 32],
            block_number: 1234,
            gas_limit: 30_000_000,
            gas_used: 21_000,
            timestamp: 1_700_000_000,
            extra_data: vec![0xAA, 0xBB].try_into().expect("extra_data fits"),
            base_fee_per_gas: [7; 32],
            block_hash: [8; 32],
            transactions: vec![vec![0xC0, 0xC1].try_into().expect("tx bytes fit")]
                .try_into()
                .expect("txs fit"),
        }
    }

    #[test]
    fn paris_payload_roundtrips() {
        let p = sample_payload();
        let bytes = p.to_ssz();
        let back = ParisPayload::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn paris_envelope_roundtrips() {
        let envelope = ParisEnvelope {
            execution_payload: sample_payload(),
        };
        let bytes = envelope.to_ssz();
        let back = ParisEnvelope::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back.execution_payload, envelope.execution_payload);
    }

    #[test]
    fn paris_attrs_roundtrip() {
        let attrs = ParisAttrs {
            timestamp: 1_700_000_001,
            prev_randao: [9; 32],
            suggested_fee_recipient: Bytes20([10; 20]),
        };
        let bytes = attrs.to_ssz();
        let back = ParisAttrs::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, attrs);
    }

    #[test]
    fn paris_payload_roundtrips_with_empty_collections() {
        let p = ParisPayload {
            parent_hash: [0; 32],
            fee_recipient: Bytes20([0; 20]),
            state_root: [0; 32],
            receipts_root: [0; 32],
            logs_bloom: vec![0; 256].try_into().unwrap(),
            prev_randao: [0; 32],
            block_number: 0,
            gas_limit: 30_000_000,
            gas_used: 0,
            timestamp: 0,
            extra_data: vec![].try_into().unwrap(),
            base_fee_per_gas: [0; 32],
            block_hash: [0; 32],
            transactions: vec![].try_into().unwrap(),
        };
        let bytes = p.to_ssz();
        let back = ParisPayload::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, p);
    }
}

mod shanghai_types_tests {
    use ethrex_rpc::engine_rest::types::common::Bytes20;
    use ethrex_rpc::engine_rest::types::shanghai::{
        ExecutionPayload as ShanghaiPayload, ExecutionPayloadEnvelope as ShanghaiEnvelope,
        PayloadAttributes as ShanghaiAttrs, Withdrawal,
    };
    use libssz::{SszDecode, SszEncode};

    fn sample_withdrawal() -> Withdrawal {
        Withdrawal {
            index: 17,
            validator_index: 7777,
            address: Bytes20([0xAB; 20]),
            amount: 32_000_000_000,
        }
    }

    fn sample_payload() -> ShanghaiPayload {
        ShanghaiPayload {
            parent_hash: [1; 32],
            fee_recipient: Bytes20([2; 20]),
            state_root: [3; 32],
            receipts_root: [4; 32],
            logs_bloom: vec![5; 256].try_into().unwrap(),
            prev_randao: [6; 32],
            block_number: 1234,
            gas_limit: 30_000_000,
            gas_used: 21_000,
            timestamp: 1_700_000_000,
            extra_data: vec![0xAA].try_into().unwrap(),
            base_fee_per_gas: [7; 32],
            block_hash: [8; 32],
            transactions: vec![vec![0xC0].try_into().unwrap()].try_into().unwrap(),
            withdrawals: vec![sample_withdrawal()].try_into().unwrap(),
        }
    }

    #[test]
    fn shanghai_payload_roundtrips_with_withdrawals() {
        let p = sample_payload();
        let bytes = p.to_ssz();
        let back = ShanghaiPayload::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.withdrawals.len(), 1);
        assert_eq!(back.withdrawals[0].validator_index, 7777);
    }

    #[test]
    fn shanghai_envelope_roundtrips() {
        let env = ShanghaiEnvelope {
            execution_payload: sample_payload(),
        };
        let bytes = env.to_ssz();
        let back = ShanghaiEnvelope::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back.execution_payload, env.execution_payload);
    }

    #[test]
    fn shanghai_attrs_roundtrip() {
        let attrs = ShanghaiAttrs {
            timestamp: 1_700_000_001,
            prev_randao: [9; 32],
            suggested_fee_recipient: Bytes20([10; 20]),
            withdrawals: vec![sample_withdrawal()].try_into().unwrap(),
        };
        let bytes = attrs.to_ssz();
        let back = ShanghaiAttrs::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, attrs);
    }
}

mod cancun_types_tests {
    use ethrex_rpc::engine_rest::types::cancun::{
        ExecutionPayload as CancunPayload, ExecutionPayloadEnvelope as CancunEnvelope,
        PayloadAttributes as CancunAttrs,
    };
    use ethrex_rpc::engine_rest::types::common::Bytes20;
    use ethrex_rpc::engine_rest::types::shanghai::Withdrawal;
    use libssz::{SszDecode, SszEncode};

    fn sample_payload() -> CancunPayload {
        CancunPayload {
            parent_hash: [1; 32],
            fee_recipient: Bytes20([2; 20]),
            state_root: [3; 32],
            receipts_root: [4; 32],
            logs_bloom: vec![5; 256].try_into().unwrap(),
            prev_randao: [6; 32],
            block_number: 1234,
            gas_limit: 30_000_000,
            gas_used: 21_000,
            timestamp: 1_700_000_000,
            extra_data: vec![0xAA].try_into().unwrap(),
            base_fee_per_gas: [7; 32],
            block_hash: [8; 32],
            transactions: vec![vec![0xC0].try_into().unwrap()].try_into().unwrap(),
            withdrawals: vec![Withdrawal {
                index: 1,
                validator_index: 2,
                address: Bytes20([3; 20]),
                amount: 4,
            }]
            .try_into()
            .unwrap(),
            blob_gas_used: 393_216,
            excess_blob_gas: 786_432,
        }
    }

    #[test]
    fn cancun_payload_roundtrips_with_blob_fields() {
        let p = sample_payload();
        let bytes = p.to_ssz();
        let back = CancunPayload::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.blob_gas_used, 393_216);
        assert_eq!(back.excess_blob_gas, 786_432);
    }

    #[test]
    fn cancun_envelope_roundtrips_with_beacon_root() {
        let env = CancunEnvelope {
            execution_payload: sample_payload(),
            parent_beacon_block_root: [0xBB; 32],
        };
        let bytes = env.to_ssz();
        let back = CancunEnvelope::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back.execution_payload, env.execution_payload);
        assert_eq!(back.parent_beacon_block_root, [0xBB; 32]);
    }

    #[test]
    fn cancun_attrs_roundtrip_with_beacon_root() {
        let attrs = CancunAttrs {
            timestamp: 1_700_000_001,
            prev_randao: [9; 32],
            suggested_fee_recipient: Bytes20([10; 20]),
            withdrawals: vec![].try_into().unwrap(),
            parent_beacon_block_root: [0xCC; 32],
        };
        let bytes = attrs.to_ssz();
        let back = CancunAttrs::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, attrs);
        assert_eq!(back.parent_beacon_block_root, [0xCC; 32]);
    }
}

mod prague_types_tests {
    use ethrex_rpc::engine_rest::types::common::Bytes20;
    use ethrex_rpc::engine_rest::types::prague::{
        ExecutionPayload as PraguePayload, ExecutionPayloadEnvelope as PragueEnvelope,
        PayloadAttributes as PragueAttrs,
    };
    use ethrex_rpc::engine_rest::types::shanghai::Withdrawal;
    use libssz::{SszDecode, SszEncode};

    fn sample_payload() -> PraguePayload {
        PraguePayload {
            parent_hash: [1; 32],
            fee_recipient: Bytes20([2; 20]),
            state_root: [3; 32],
            receipts_root: [4; 32],
            logs_bloom: vec![5; 256].try_into().unwrap(),
            prev_randao: [6; 32],
            block_number: 1234,
            gas_limit: 30_000_000,
            gas_used: 21_000,
            timestamp: 1_700_000_000,
            extra_data: vec![0xAA].try_into().unwrap(),
            base_fee_per_gas: [7; 32],
            block_hash: [8; 32],
            transactions: vec![vec![0xC0].try_into().unwrap()].try_into().unwrap(),
            withdrawals: vec![Withdrawal {
                index: 1,
                validator_index: 2,
                address: Bytes20([3; 20]),
                amount: 4,
            }]
            .try_into()
            .unwrap(),
            blob_gas_used: 0,
            excess_blob_gas: 0,
        }
    }

    #[test]
    fn prague_envelope_roundtrips_with_execution_requests() {
        let env = PragueEnvelope {
            execution_payload: sample_payload(),
            parent_beacon_block_root: [0xBB; 32],
            execution_requests: vec![
                vec![0x00, 0xDE, 0xAD].try_into().unwrap(), // deposit prefix
                vec![0x01, 0xBE, 0xEF].try_into().unwrap(), // withdrawal prefix
            ]
            .try_into()
            .unwrap(),
        };
        let bytes = env.to_ssz();
        let back = PragueEnvelope::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back.execution_requests.len(), 2);
        assert_eq!(back.execution_requests[0][0], 0x00);
        assert_eq!(back.execution_requests[1][0], 0x01);
    }

    #[test]
    fn prague_attrs_roundtrip() {
        let attrs = PragueAttrs {
            timestamp: 1_700_000_001,
            prev_randao: [9; 32],
            suggested_fee_recipient: Bytes20([10; 20]),
            withdrawals: vec![].try_into().unwrap(),
            parent_beacon_block_root: [0xCC; 32],
        };
        let bytes = attrs.to_ssz();
        let back = PragueAttrs::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, attrs);
    }
}

mod osaka_types_tests {
    use ethrex_rpc::engine_rest::types::common::Bytes20;
    use ethrex_rpc::engine_rest::types::osaka::{
        ExecutionPayload as OsakaPayload, ExecutionPayloadEnvelope as OsakaEnvelope,
        PayloadAttributes as OsakaAttrs,
    };

    #[test]
    fn osaka_payload_is_type_alias_for_prague_via_pub_use() {
        // Compile-time check: a Prague-shaped payload assigns into the Osaka alias.
        let _: OsakaPayload = ethrex_rpc::engine_rest::types::prague::ExecutionPayload {
            parent_hash: [0; 32],
            fee_recipient: Bytes20([0; 20]),
            state_root: [0; 32],
            receipts_root: [0; 32],
            logs_bloom: vec![0; 256].try_into().unwrap(),
            prev_randao: [0; 32],
            block_number: 0,
            gas_limit: 0,
            gas_used: 0,
            timestamp: 0,
            extra_data: vec![].try_into().unwrap(),
            base_fee_per_gas: [0; 32],
            block_hash: [0; 32],
            transactions: vec![].try_into().unwrap(),
            withdrawals: vec![].try_into().unwrap(),
            blob_gas_used: 0,
            excess_blob_gas: 0,
        };
    }

    #[test]
    fn osaka_envelope_and_attrs_are_aliases() {
        let _: OsakaEnvelope = ethrex_rpc::engine_rest::types::prague::ExecutionPayloadEnvelope {
            execution_payload: ethrex_rpc::engine_rest::types::prague::ExecutionPayload {
                parent_hash: [0; 32],
                fee_recipient: Bytes20([0; 20]),
                state_root: [0; 32],
                receipts_root: [0; 32],
                logs_bloom: vec![0; 256].try_into().unwrap(),
                prev_randao: [0; 32],
                block_number: 0,
                gas_limit: 0,
                gas_used: 0,
                timestamp: 0,
                extra_data: vec![].try_into().unwrap(),
                base_fee_per_gas: [0; 32],
                block_hash: [0; 32],
                transactions: vec![].try_into().unwrap(),
                withdrawals: vec![].try_into().unwrap(),
                blob_gas_used: 0,
                excess_blob_gas: 0,
            },
            parent_beacon_block_root: [0; 32],
            execution_requests: vec![].try_into().unwrap(),
        };
        let _: OsakaAttrs = ethrex_rpc::engine_rest::types::prague::PayloadAttributes {
            timestamp: 0,
            prev_randao: [0; 32],
            suggested_fee_recipient: Bytes20([0; 20]),
            withdrawals: vec![].try_into().unwrap(),
            parent_beacon_block_root: [0; 32],
        };
    }
}

mod amsterdam_types_tests {
    use ethrex_rpc::engine_rest::types::amsterdam::{
        ExecutionPayload as AmsterdamPayload, ExecutionPayloadEnvelope as AmsterdamEnvelope,
        PayloadAttributes as AmsterdamAttrs,
    };
    use ethrex_rpc::engine_rest::types::common::Bytes20;
    use ethrex_rpc::engine_rest::types::shanghai::Withdrawal;
    use libssz::{SszDecode, SszEncode};

    fn sample_payload() -> AmsterdamPayload {
        AmsterdamPayload {
            parent_hash: [1; 32],
            fee_recipient: Bytes20([2; 20]),
            state_root: [3; 32],
            receipts_root: [4; 32],
            logs_bloom: vec![5; 256].try_into().unwrap(),
            prev_randao: [6; 32],
            block_number: 1234,
            gas_limit: 30_000_000,
            gas_used: 21_000,
            timestamp: 1_700_000_000,
            extra_data: vec![0xAA].try_into().unwrap(),
            base_fee_per_gas: [7; 32],
            block_hash: [8; 32],
            transactions: vec![vec![0xC0].try_into().unwrap()].try_into().unwrap(),
            withdrawals: vec![Withdrawal {
                index: 1,
                validator_index: 2,
                address: Bytes20([3; 20]),
                amount: 4,
            }]
            .try_into()
            .unwrap(),
            blob_gas_used: 0,
            excess_blob_gas: 0,
            block_access_list: vec![0xCA, 0xFE, 0xBA, 0xBE].try_into().unwrap(),
            slot_number: 42_000_000,
        }
    }

    #[test]
    fn amsterdam_payload_roundtrips_with_bal_and_slot() {
        let p = sample_payload();
        let bytes = p.to_ssz();
        let back = AmsterdamPayload::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, p);
        assert_eq!(&back.block_access_list[..], &[0xCA, 0xFE, 0xBA, 0xBE]);
        assert_eq!(back.slot_number, 42_000_000);
    }

    #[test]
    fn amsterdam_envelope_roundtrips() {
        let env = AmsterdamEnvelope {
            execution_payload: sample_payload(),
            parent_beacon_block_root: [0xBB; 32],
            execution_requests: vec![vec![0x00, 0xDE, 0xAD].try_into().unwrap()]
                .try_into()
                .unwrap(),
        };
        let bytes = env.to_ssz();
        let back = AmsterdamEnvelope::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back.execution_payload.slot_number, 42_000_000);
        assert_eq!(back.execution_requests.len(), 1);
    }

    #[test]
    fn amsterdam_attrs_roundtrip_with_slot_and_target_gas() {
        let attrs = AmsterdamAttrs {
            timestamp: 1_700_000_001,
            prev_randao: [9; 32],
            suggested_fee_recipient: Bytes20([10; 20]),
            withdrawals: vec![].try_into().unwrap(),
            parent_beacon_block_root: [0xCC; 32],
            slot_number: 42_000_001,
            target_gas_limit: 36_000_000,
        };
        let bytes = attrs.to_ssz();
        let back = AmsterdamAttrs::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, attrs);
        assert_eq!(back.slot_number, 42_000_001);
        assert_eq!(back.target_gas_limit, 36_000_000);
    }
}

mod conversion_tests {
    use ethrex_rpc::engine_rest::types::common::Bytes20;
    use ethrex_rpc::engine_rest::types::conversions::{
        DecodedNewPayload, EngineCall, IntoEngineCall,
    };

    fn paris_empty_envelope() -> ethrex_rpc::engine_rest::types::paris::ExecutionPayloadEnvelope {
        use ethrex_rpc::engine_rest::types::paris::*;
        ExecutionPayloadEnvelope {
            execution_payload: ExecutionPayload {
                parent_hash: [0; 32],
                fee_recipient: Bytes20([0; 20]),
                state_root: [0; 32],
                receipts_root: [0; 32],
                logs_bloom: vec![0; 256].try_into().unwrap(),
                prev_randao: [0; 32],
                block_number: 0,
                gas_limit: 30_000_000,
                gas_used: 0,
                timestamp: 0,
                extra_data: vec![].try_into().unwrap(),
                base_fee_per_gas: {
                    let mut a = [0u8; 32];
                    a[0] = 0x07; // small little-endian value (7 wei)
                    a
                },
                block_hash: [0; 32],
                transactions: vec![].try_into().unwrap(),
            },
        }
    }

    #[test]
    fn paris_envelope_dispatches_to_v1v2() {
        let env = paris_empty_envelope();
        let DecodedNewPayload { block, call, .. } = env.into_engine_call().expect("conversion");
        assert!(matches!(call, EngineCall::V1V2));
        assert_eq!(block.header.gas_limit, 30_000_000);
        assert_eq!(block.header.base_fee_per_gas, Some(7));
    }

    #[test]
    fn cancun_envelope_dispatches_to_v3_with_beacon_root() {
        use ethrex_rpc::engine_rest::types::cancun::*;
        let env = ExecutionPayloadEnvelope {
            execution_payload: ExecutionPayload {
                parent_hash: [0; 32],
                fee_recipient: Bytes20([0; 20]),
                state_root: [0; 32],
                receipts_root: [0; 32],
                logs_bloom: vec![0; 256].try_into().unwrap(),
                prev_randao: [0; 32],
                block_number: 0,
                gas_limit: 30_000_000,
                gas_used: 0,
                timestamp: 0,
                extra_data: vec![].try_into().unwrap(),
                base_fee_per_gas: [0; 32],
                block_hash: [0; 32],
                transactions: vec![].try_into().unwrap(),
                withdrawals: vec![].try_into().unwrap(),
                blob_gas_used: 0,
                excess_blob_gas: 0,
            },
            parent_beacon_block_root: [0xBB; 32],
        };
        let DecodedNewPayload { block, call, .. } = env.into_engine_call().expect("conversion");
        assert!(matches!(call, EngineCall::V3), "expected V3, got {call:?}");
        // parent_beacon_block_root is baked into the reconstructed header.
        assert_eq!(
            block.header.parent_beacon_block_root,
            Some(ethrex_common::H256::from([0xBB; 32]))
        );
    }

    #[test]
    fn prague_envelope_dispatches_to_v4_with_requests() {
        use ethrex_rpc::engine_rest::types::prague::*;
        let env = ExecutionPayloadEnvelope {
            execution_payload: ExecutionPayload {
                parent_hash: [0; 32],
                fee_recipient: Bytes20([0; 20]),
                state_root: [0; 32],
                receipts_root: [0; 32],
                logs_bloom: vec![0; 256].try_into().unwrap(),
                prev_randao: [0; 32],
                block_number: 0,
                gas_limit: 30_000_000,
                gas_used: 0,
                timestamp: 0,
                extra_data: vec![].try_into().unwrap(),
                base_fee_per_gas: [0; 32],
                block_hash: [0; 32],
                transactions: vec![].try_into().unwrap(),
                withdrawals: vec![].try_into().unwrap(),
                blob_gas_used: 0,
                excess_blob_gas: 0,
            },
            parent_beacon_block_root: [0xBB; 32],
            execution_requests: vec![vec![0x00u8, 0xDE, 0xAD].try_into().unwrap()]
                .try_into()
                .unwrap(),
        };
        let DecodedNewPayload { block, call, .. } = env.into_engine_call().expect("conversion");
        assert!(matches!(call, EngineCall::V4), "expected V4, got {call:?}");
        // The decoded execution_requests are folded into the header's requests_hash;
        // assert on that observable state rather than on a now-removed enum field.
        let expected = ethrex_common::types::requests::compute_requests_hash(&[
            ethrex_common::types::requests::EncodedRequests(bytes::Bytes::from(vec![
                0x00u8, 0xDE, 0xAD,
            ])),
        ]);
        assert_eq!(block.header.requests_hash, Some(expected));
    }

    #[test]
    fn shanghai_envelope_dispatches_to_v1v2_no_blob_fields() {
        use ethrex_rpc::engine_rest::types::shanghai::*;
        let env = ExecutionPayloadEnvelope {
            execution_payload: ExecutionPayload {
                parent_hash: [0; 32],
                fee_recipient: Bytes20([0; 20]),
                state_root: [0; 32],
                receipts_root: [0; 32],
                logs_bloom: vec![0; 256].try_into().unwrap(),
                prev_randao: [0; 32],
                block_number: 0,
                gas_limit: 30_000_000,
                gas_used: 0,
                timestamp: 0,
                extra_data: vec![].try_into().unwrap(),
                base_fee_per_gas: [0; 32],
                block_hash: [0; 32],
                transactions: vec![].try_into().unwrap(),
                withdrawals: vec![Withdrawal {
                    index: 1,
                    validator_index: 2,
                    address: Bytes20([3; 20]),
                    amount: 4,
                }]
                .try_into()
                .unwrap(),
            },
        };
        let DecodedNewPayload { block, call, .. } = env.into_engine_call().expect("conversion");
        assert!(matches!(call, EngineCall::V1V2));
        // Critical: Shanghai blocks MUST have None for blob fields, not Some(0).
        assert_eq!(block.header.blob_gas_used, None);
        assert_eq!(block.header.excess_blob_gas, None);
        assert!(
            block.body.withdrawals.is_some(),
            "Shanghai must carry withdrawals"
        );
    }

    #[test]
    fn amsterdam_envelope_dispatches_to_v5_with_bal() {
        use ethrex_common::types::block_access_list::BlockAccessList;
        use ethrex_rlp::encode::RLPEncode;
        use ethrex_rpc::engine_rest::types::amsterdam::*;
        // The block_access_list field carries the RLP-encoded BAL; the conversion
        // decodes it (to run validate_ordering downstream), so it must be valid RLP.
        let bal_rlp = BlockAccessList::new().encode_to_vec();
        let env = ExecutionPayloadEnvelope {
            execution_payload: ExecutionPayload {
                parent_hash: [0; 32],
                fee_recipient: Bytes20([0; 20]),
                state_root: [0; 32],
                receipts_root: [0; 32],
                logs_bloom: vec![0; 256].try_into().unwrap(),
                prev_randao: [0; 32],
                block_number: 0,
                gas_limit: 30_000_000,
                gas_used: 0,
                timestamp: 0,
                extra_data: vec![].try_into().unwrap(),
                base_fee_per_gas: [0; 32],
                block_hash: [0; 32],
                transactions: vec![].try_into().unwrap(),
                withdrawals: vec![].try_into().unwrap(),
                blob_gas_used: 0,
                excess_blob_gas: 0,
                block_access_list: bal_rlp.try_into().unwrap(),
                slot_number: 100,
            },
            parent_beacon_block_root: [0xBB; 32],
            execution_requests: vec![].try_into().unwrap(),
        };
        let DecodedNewPayload { block, call, .. } = env.into_engine_call().expect("conversion");
        match call {
            EngineCall::V5 { raw_bal_hash, .. } => {
                assert!(raw_bal_hash.is_some(), "BAL hash should be precomputed");
            }
            other => panic!("expected V5, got {other:?}"),
        }
        assert_eq!(block.header.slot_number, Some(100));
    }
}

mod submit_payload_tests {
    use super::test_helpers::auth_token;
    use axum::http::StatusCode;
    use bytes::Bytes;
    use ethrex_rpc::engine_rest::router;
    use ethrex_rpc::engine_rest::types::cancun::{
        ExecutionPayload as CancunPayload, ExecutionPayloadEnvelope as CancunEnv,
    };
    use ethrex_rpc::engine_rest::types::common::Bytes20;
    use ethrex_rpc::test_utils::default_context_with_storage;
    use ethrex_rpc::test_utils::setup_store;
    use libssz::SszEncode;
    use tower::ServiceExt;

    fn empty_cancun_envelope() -> CancunEnv {
        CancunEnv {
            execution_payload: CancunPayload {
                parent_hash: [0; 32],
                fee_recipient: Bytes20([0; 20]),
                state_root: [0; 32],
                receipts_root: [0; 32],
                logs_bloom: vec![0; 256].try_into().unwrap(),
                prev_randao: [0; 32],
                block_number: 0,
                gas_limit: 30_000_000,
                gas_used: 0,
                timestamp: 0,
                extra_data: vec![].try_into().unwrap(),
                base_fee_per_gas: [0; 32],
                block_hash: [0; 32],
                transactions: vec![].try_into().unwrap(),
                withdrawals: vec![].try_into().unwrap(),
                blob_gas_used: 0,
                excess_blob_gas: 0,
            },
            parent_beacon_block_root: [0; 32],
        }
    }

    /// On the test genesis Osaka is active from timestamp 0, so a Cancun-shaped
    /// payload (timestamp 0 → Osaka era) submitted to `/cancun/payloads` is
    /// misrouted. The fork-boundary check (mirroring JSON-RPC NewPayloadV3
    /// `validate_fork(Cancun)`) rejects it with 400 instead of letting it fall
    /// through to an INVALID block-hash mismatch.
    #[tokio::test]
    async fn submit_wrong_fork_payload_returns_400() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let env = empty_cancun_envelope();
        let body = env.to_ssz();
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/cancun/payloads")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 400);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
    }

    #[tokio::test]
    async fn submit_malformed_ssz_returns_400_problem_json() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/cancun/payloads")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(vec![0xFFu8; 10])) // not a valid envelope
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
    }
}

mod get_payload_tests {
    use super::test_helpers::auth_token;
    use bytes::Bytes;
    use ethrex_rpc::engine_rest::router;
    use ethrex_rpc::test_utils::default_context_with_storage;
    use ethrex_rpc::test_utils::setup_store;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn unknown_payload_id_returns_404() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/cancun/payloads/0x0102030405060708")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 404);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["status"], 404);
    }

    #[tokio::test]
    async fn malformed_payload_id_returns_400() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/cancun/payloads/not-hex")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 400);
    }
}

mod forkchoice_handler_tests {
    use super::test_helpers::auth_token;
    use bytes::Bytes;
    use ethrex_rpc::engine_rest::router;
    use ethrex_rpc::engine_rest::types::common::{
        ForkchoiceResponse, ForkchoiceState, to_optional,
    };
    use ethrex_rpc::engine_rest::types::forkchoice_update::CancunForkchoiceUpdate;
    use ethrex_rpc::test_utils::default_context_with_storage;
    use ethrex_rpc::test_utils::setup_store;
    use http_body_util::BodyExt;
    use libssz::{SszDecode, SszEncode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn state_only_update_with_unknown_head_returns_syncing() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let update = CancunForkchoiceUpdate {
            state: ForkchoiceState {
                head_block_hash: [0xFF; 32], // unknown head
                safe_block_hash: [0; 32],
                finalized_block_hash: [0; 32],
            },
            payload_attributes: to_optional(None),
        };
        let body = update.to_ssz();
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/cancun/forkchoice")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let r = ForkchoiceResponse::from_ssz_bytes(&body_bytes).unwrap();
        // Unknown head → SYNCING (status code 2).
        assert_eq!(
            r.payload_status.status,
            ethrex_rpc::engine_rest::types::common::PayloadStatusCode::Syncing as u8
        );
        assert!(r.payload_id().is_none());
    }

    #[tokio::test]
    async fn malformed_ssz_returns_400() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/cancun/forkchoice")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(vec![0xFFu8; 10]))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 400);
    }

    /// Regression: a forkchoice update carrying payload attributes with a
    /// non-advancing timestamp must be rejected (422), mirroring the JSON-RPC
    /// `ForkChoiceUpdatedV3` path (`-38003 InvalidPayloadAttributes`). Before the
    /// validation-parity fix the REST path built a payload and returned 200 + a
    /// payloadId for these attributes.
    ///
    /// Attribute validation only runs when the forkchoice resolves to a known
    /// head, so we build a block on genesis, store it non-canonically, and point
    /// the FCU at it (apply_fork_choice then advances the head). Mirrors the
    /// fixture in `fork_choice_tests.rs`.
    #[tokio::test]
    async fn stale_timestamp_attributes_rejected_with_422() {
        use std::fs::File;
        use std::io::BufReader;
        use std::path::PathBuf;

        use ethrex_blockchain::Blockchain;
        use ethrex_blockchain::payload::{BuildPayloadArgs, create_payload};
        use ethrex_common::types::{DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER};
        use ethrex_common::{H160, H256};
        use ethrex_rpc::engine_rest::types::cancun::PayloadAttributes;
        use ethrex_rpc::engine_rest::types::common::Bytes20;
        use ethrex_storage::{EngineType, Store};

        // Genesis with Cancun active from 0 (Prague tip, no Osaka), matching the
        // JSON-RPC fork_choice tests — create_payload/build_payload produce a valid
        // block and the V3 attribute validator reaches the timestamp check.
        let genesis_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures/genesis/execution-api.json");
        let genesis = serde_json::from_reader(BufReader::new(
            File::open(genesis_path).expect("open genesis"),
        ))
        .expect("parse genesis");
        let mut storage = Store::new("", EngineType::InMemory).expect("store");
        storage
            .add_initial_state(genesis)
            .await
            .expect("genesis state");

        let blockchain = Blockchain::default_with_store(storage.clone());
        let genesis_header = storage.get_block_header(0).unwrap().unwrap();

        // Build a block on genesis and store it WITHOUT canonicalizing, so the FCU
        // advances the head (apply_fork_choice returns Some(head)) and validation runs.
        let args = BuildPayloadArgs {
            parent: genesis_header.hash(),
            timestamp: genesis_header.timestamp + 12,
            fee_recipient: H160::random(),
            random: H256::random(),
            withdrawals: Some(Vec::new()),
            beacon_root: Some(H256::random()),
            slot_number: None,
            version: 3,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        };
        let payload = create_payload(&args, &storage, Bytes::new()).unwrap();
        let block_1 = blockchain.build_payload(payload).unwrap().payload;
        let hash_1 = block_1.hash();
        blockchain.add_block(block_1.clone()).unwrap();

        let mut ctx = default_context_with_storage(storage.clone()).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        // head advances to block_1; attrs.timestamp == block_1.timestamp is stale
        // (the engine API requires strictly greater), so validation must reject
        // (422) instead of building a payload.
        let update = CancunForkchoiceUpdate {
            state: ForkchoiceState {
                head_block_hash: hash_1.0,
                safe_block_hash: genesis_header.hash().0,
                finalized_block_hash: genesis_header.hash().0,
            },
            payload_attributes: to_optional(Some(PayloadAttributes {
                timestamp: block_1.header.timestamp,
                prev_randao: [0; 32],
                suggested_fee_recipient: Bytes20([0xFE; 20]),
                withdrawals: vec![].try_into().unwrap(),
                parent_beacon_block_root: [0xBB; 32],
            })),
        };
        let body = update.to_ssz();
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/cancun/forkchoice")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            422,
            "stale-timestamp attributes must be rejected, not built"
        );
    }
}

mod end_to_end_tests {
    use super::test_helpers::auth_token;
    use bytes::Bytes;
    use ethrex_rpc::engine_rest::router;
    use ethrex_rpc::engine_rest::types::built_payload::BuiltPayloadCancun;
    use ethrex_rpc::engine_rest::types::cancun::PayloadAttributes;
    use ethrex_rpc::engine_rest::types::common::{
        Bytes20, ForkchoiceResponse, ForkchoiceState, to_optional,
    };
    use ethrex_rpc::engine_rest::types::forkchoice_update::CancunForkchoiceUpdate;
    use ethrex_rpc::test_utils::{
        add_eip1559_tx_blocks, default_context_with_storage, setup_store,
    };
    use http_body_util::BodyExt;
    use libssz::{SszDecode, SszEncode};
    use tower::ServiceExt;

    /// Submit a forkchoice with payload_attributes pointing at the seeded head,
    /// then GET the resulting payloadId to verify the envelope round-trips.
    ///
    /// `#[ignore]`d by default because reliably triggering a build that
    /// produces a non-empty envelope requires deeper fixture setup than the
    /// `default_context_with_storage` helper offers (e.g. a live payload builder
    /// and beacon block root plumbing). A non-ignored version lands in
    /// sub-project 4 alongside the measurement harness.
    #[tokio::test]
    #[ignore = "fixture infra deferred to sub-project 4"]
    async fn cancun_build_then_get_payload_round_trip() {
        let storage = setup_store().await;
        // Seed a small chain (3 blocks of EIP-1559 txs).
        add_eip1559_tx_blocks(&storage, 3, 2).await;
        let mut ctx = default_context_with_storage(storage.clone()).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx.clone());
        let token = auth_token(&secret).await;

        // Look up the head — last block's hash.
        let head_hash = storage
            .get_latest_canonical_block_hash()
            .await
            .unwrap()
            .unwrap();
        let head_header = storage
            .get_block_header_by_hash(head_hash)
            .unwrap()
            .unwrap();

        // POST /cancun/forkchoice with payload_attributes.
        let update = CancunForkchoiceUpdate {
            state: ForkchoiceState {
                head_block_hash: head_hash.0,
                safe_block_hash: head_hash.0,
                finalized_block_hash: head_hash.0,
            },
            payload_attributes: to_optional(Some(PayloadAttributes {
                timestamp: head_header.timestamp + 12,
                prev_randao: [0; 32],
                suggested_fee_recipient: Bytes20([0xFE; 20]),
                withdrawals: vec![].try_into().unwrap(),
                parent_beacon_block_root: [0xBB; 32],
            })),
        };
        let body = update.to_ssz();
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/cancun/forkchoice")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let fcr = ForkchoiceResponse::from_ssz_bytes(&body_bytes).unwrap();
        let payload_id = fcr
            .payload_id()
            .expect("payload_id should be Some when attrs provided");

        // GET /cancun/payloads/{id} — verify the SSZ BuiltPayload decodes.
        let req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/cancun/payloads/{}", payload_id.to_hex_string()))
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let built = BuiltPayloadCancun::from_ssz_bytes(&body_bytes).unwrap();
        assert!(!built.should_override_builder);
    }
}

mod helpers_tests {
    use axum::http::HeaderMap;
    use ethrex_rpc::engine_rest::extractors::decode_ssz;
    use ethrex_rpc::engine_rest::handlers::helpers::check_content_type;
    use libssz::SszEncode;
    use libssz_derive::{SszDecode, SszEncode};

    #[derive(Debug, PartialEq, Eq, SszEncode, SszDecode)]
    struct Sample {
        a: u32,
    }

    #[test]
    fn decode_ssz_returns_400_on_malformed_bytes() {
        let result: Result<Sample, _> = decode_ssz(&[0xFFu8; 2]);
        assert!(result.is_err());
        let problem = result.unwrap_err();
        assert_eq!(problem.status, 400);
    }

    #[test]
    fn decode_ssz_round_trips_valid_bytes() {
        let s = Sample { a: 0xDEADBEEF };
        let bytes = s.to_ssz();
        let back: Sample = decode_ssz(&bytes).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn check_content_type_accepts_octet_stream() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "application/octet-stream".parse().unwrap());
        assert!(check_content_type(&headers).is_ok());
    }

    #[test]
    fn check_content_type_rejects_json_with_415() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        let problem = check_content_type(&headers).unwrap_err();
        assert_eq!(problem.status, 415);
    }
}

mod bodies_types_tests {
    use ethrex_rpc::engine_rest::types::bodies::{
        BlockHashList, BodyAmsterdam, BodyParis, BodyShanghai,
    };
    use ethrex_rpc::engine_rest::types::common::Bytes20;
    use ethrex_rpc::engine_rest::types::shanghai::Withdrawal;
    use libssz::{SszDecode, SszEncode};

    #[test]
    fn body_paris_roundtrips_empty() {
        let body = BodyParis {
            transactions: vec![].try_into().unwrap(),
        };
        let bytes = body.to_ssz();
        let back = BodyParis::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, body);
    }

    #[test]
    fn body_shanghai_roundtrips_with_withdrawals() {
        let body = BodyShanghai {
            transactions: vec![vec![0xC0].try_into().unwrap()].try_into().unwrap(),
            withdrawals: vec![Withdrawal {
                index: 1,
                validator_index: 2,
                address: Bytes20([3; 20]),
                amount: 4,
            }]
            .try_into()
            .unwrap(),
        };
        let bytes = body.to_ssz();
        let back = BodyShanghai::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, body);
    }

    #[test]
    fn body_amsterdam_roundtrips_with_bal() {
        let body = BodyAmsterdam {
            transactions: vec![].try_into().unwrap(),
            withdrawals: vec![].try_into().unwrap(),
            block_access_list: vec![0xCA, 0xFE].try_into().unwrap(),
        };
        let bytes = body.to_ssz();
        let back = BodyAmsterdam::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, body);
        assert_eq!(&back.block_access_list[..], &[0xCA, 0xFE]);
    }

    #[test]
    fn bodies_by_hash_request_roundtrips() {
        let req: BlockHashList = vec![[1u8; 32], [2u8; 32]].try_into().unwrap();
        let bytes = req.to_ssz();
        let back = BlockHashList::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0], [1u8; 32]);
    }
}

mod bodies_by_hash_tests {
    use super::test_helpers::auth_token;
    use axum::http::StatusCode;
    use bytes::Bytes;
    use ethrex_rpc::engine_rest::router;
    use ethrex_rpc::engine_rest::types::bodies::BlockHashList;
    use ethrex_rpc::test_utils::{
        add_eip1559_tx_blocks, default_context_with_storage, setup_store,
    };
    use http_body_util::BodyExt;
    use libssz::SszEncode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn returns_200_for_known_and_unknown_hashes() {
        let storage = setup_store().await;
        add_eip1559_tx_blocks(&storage, 3, 2).await;
        let mut ctx = default_context_with_storage(storage.clone()).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        // Look up block 1's hash.
        let block1_hash = storage
            .get_canonical_block_hash(1)
            .await
            .unwrap()
            .expect("block 1 should exist");

        let req_body: BlockHashList = vec![block1_hash.0, [0xFFu8; 32]].try_into().unwrap();
        let body = req_body.to_ssz();

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/cancun/bodies/hash")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/octet-stream"
        );
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(
            !body_bytes.is_empty(),
            "response body should be non-empty SSZ"
        );
    }

    #[tokio::test]
    async fn missing_content_type_returns_415() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let req_body: BlockHashList = vec![].try_into().unwrap();
        let body = req_body.to_ssz();

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/cancun/bodies/hash")
            .header("authorization", format!("Bearer {token}"))
            // NO content-type
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn unsupported_fork_returns_400() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let req_body: BlockHashList = vec![].try_into().unwrap();
        let body = req_body.to_ssz();

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/frontier/bodies/hash")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

mod bodies_by_range_tests {
    use super::test_helpers::auth_token;
    use axum::http::StatusCode;
    use bytes::Bytes;
    use ethrex_rpc::engine_rest::router;
    use ethrex_rpc::test_utils::{
        add_eip1559_tx_blocks, default_context_with_storage, setup_store,
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn returns_seeded_blocks_in_range() {
        let storage = setup_store().await;
        add_eip1559_tx_blocks(&storage, 5, 1).await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/cancun/bodies?from=1&count=3")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/octet-stream"
        );
    }

    #[tokio::test]
    async fn missing_query_params_returns_400() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/cancun/bodies") // no from/count
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn count_zero_returns_400() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/cancun/bodies?from=1&count=0")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn count_exceeds_cap_returns_413() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/cancun/bodies?from=1&count=129")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}

mod blobs_types_tests {
    use ethrex_rpc::engine_rest::types::blobs::{
        BlobAndProofV1, BlobAndProofV2, BlobsRequestV4, VersionedHashList,
    };
    use libssz::{SszDecode, SszEncode};

    #[test]
    fn blobs_request_roundtrips() {
        let req: VersionedHashList = vec![[1u8; 32], [2u8; 32]].try_into().unwrap();
        let bytes = req.to_ssz();
        let back = VersionedHashList::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back.len(), 2);
    }

    #[test]
    fn blobs_request_v4_roundtrips_with_indices_bitarray() {
        use libssz_types::SszBitvector;
        let mut bits = SszBitvector::<128>::new();
        bits.set(0, true).unwrap();
        bits.set(5, true).unwrap();
        bits.set(32, true).unwrap();
        let req = BlobsRequestV4 {
            versioned_hashes: vec![[3u8; 32]].try_into().unwrap(),
            indices_bitarray: bits.clone(),
        };
        let bytes = req.to_ssz();
        let back = BlobsRequestV4::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back.versioned_hashes.len(), 1);
        assert_eq!(back.indices_bitarray, bits);
        assert_eq!(back.indices_bitarray.get(5), Some(true));
        assert_eq!(back.indices_bitarray.get(6), Some(false));
    }

    #[test]
    fn blob_and_proof_v1_roundtrips() {
        let v1 = BlobAndProofV1 {
            blob: vec![0xAAu8; 131_072].try_into().unwrap(),
            proof: [0xBB; 48],
        };
        let bytes = v1.to_ssz();
        let back = BlobAndProofV1::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, v1);
    }

    #[test]
    fn blob_and_proof_v2_roundtrips_with_multiple_proofs() {
        let v2 = BlobAndProofV2 {
            blob: vec![0xAAu8; 131_072].try_into().unwrap(),
            proofs: vec![[0xBB; 48], [0xCC; 48]].try_into().unwrap(),
        };
        let bytes = v2.to_ssz();
        let back = BlobAndProofV2::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back.proofs.len(), 2);
    }

    #[test]
    fn blobs_v1_response_roundtrips_mixed_availability() {
        use ethrex_rpc::engine_rest::types::blobs::{BlobV1Entry, BlobsV1Response};
        let entries = vec![
            BlobV1Entry::available(BlobAndProofV1 {
                blob: vec![0x11u8; 131_072].try_into().unwrap(),
                proof: [0x22; 48],
            }),
            BlobV1Entry::unavailable(),
        ];
        let resp: BlobsV1Response = entries.try_into().unwrap();
        let bytes = resp.to_ssz();
        let back = BlobsV1Response::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, resp);
        assert_eq!(back.len(), 2);
    }

    #[test]
    fn blobs_v4_response_roundtrips_with_optional_cells() {
        use ethrex_rpc::engine_rest::types::blobs::{
            BYTES_PER_CELL, BlobCellsAndProofs, BlobV4Entry, BlobsV4Response,
        };
        use libssz_types::{SszList, SszVector};
        // Optional[T] ≡ List[T, 1]: empty inner list = absent, one element = present.
        let absent_cell: SszList<SszVector<u8, BYTES_PER_CELL>, 1> = Vec::new().try_into().unwrap();
        let present_cell: SszList<SszVector<u8, BYTES_PER_CELL>, 1> =
            vec![vec![0x33u8; BYTES_PER_CELL].try_into().unwrap()]
                .try_into()
                .unwrap();
        let absent_proof: SszList<[u8; 48], 1> = Vec::new().try_into().unwrap();
        let present_proof: SszList<[u8; 48], 1> = vec![[0x44u8; 48]].try_into().unwrap();
        let entry = BlobV4Entry {
            available: true,
            contents: BlobCellsAndProofs {
                blob_cells: vec![absent_cell, present_cell].try_into().unwrap(),
                proofs: vec![absent_proof, present_proof].try_into().unwrap(),
            },
        };
        let resp: BlobsV4Response = vec![entry].try_into().unwrap();
        let bytes = resp.to_ssz();
        let back = BlobsV4Response::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, resp);
    }
}

mod blobs_v1_tests {
    use super::test_helpers::auth_token;
    use bytes::Bytes;
    use ethrex_rpc::engine_rest::router;
    use ethrex_rpc::engine_rest::types::blobs::VersionedHashList;
    use ethrex_rpc::test_utils::{default_context_with_storage, setup_store};
    use libssz::SszEncode;
    use tower::ServiceExt;

    /// The test genesis activates Osaka from timestamp 0, so the canonical head is
    /// Osaka-era. `/blobs/v1` serves whole-blob proofs and is only valid pre-Osaka
    /// (mirrors JSON-RPC `getBlobsV1`); post-Osaka it must reject with 400 rather
    /// than return a `proofs[0]` that, for a cell-proof blob, would be a cell proof
    /// and fail KZG verification at the CL. Pre-Osaka per-entry behavior is covered
    /// by the v3 path.
    #[tokio::test]
    async fn post_osaka_returns_400_unsupported() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let req_body: VersionedHashList = vec![[0xFFu8; 32]].try_into().unwrap();
        let body = req_body.to_ssz();
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/blobs/v1")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 400);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
    }

    #[tokio::test]
    async fn missing_content_type_returns_415() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let req_body: VersionedHashList = vec![].try_into().unwrap();
        let body = req_body.to_ssz();
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/blobs/v1")
            .header("authorization", format!("Bearer {token}"))
            // NO content-type
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 415);
    }
}

mod blobs_v2v3_tests {
    use super::test_helpers::auth_token;
    use bytes::Bytes;
    use ethrex_rpc::engine_rest::router;
    use ethrex_rpc::engine_rest::types::blobs::VersionedHashList;
    use ethrex_rpc::test_utils::{default_context_with_storage, setup_store};
    use libssz::SszEncode;
    use tower::ServiceExt;

    async fn build_app() -> (axum::Router, Bytes) {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        (router(ctx), secret)
    }

    #[tokio::test]
    async fn v2_unknown_hash_returns_204_all_or_nothing() {
        // V2 is all-or-nothing: a missing blob means the EL returns 204 No Content
        // (execution-apis #793 §POST /blobs/v2), not a per-slot null.
        let (app, secret) = build_app().await;
        let token = auth_token(&secret).await;
        let req_body: VersionedHashList = vec![[0xFFu8; 32]].try_into().unwrap();
        let body = req_body.to_ssz();
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/blobs/v2")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 204);
    }

    #[tokio::test]
    async fn v3_unknown_hash_returns_200_with_none_position() {
        let (app, secret) = build_app().await;
        let token = auth_token(&secret).await;
        let req_body: VersionedHashList = vec![[0xFFu8; 32]].try_into().unwrap();
        let body = req_body.to_ssz();
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/blobs/v3")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn v2_exceeds_count_returns_400_or_413() {
        let (app, secret) = build_app().await;
        let token = auth_token(&secret).await;
        // The request is a bare `List[Bytes32, 128]`, whose SSZ encoding is just the
        // concatenated 32-byte elements (fixed-size element list — no offsets). 129
        // elements (129 * 32 bytes) exceeds the cap, so the SSZ extractor rejects the
        // body (400) before the handler, and `VersionedHashList::try_into` can't build
        // an over-cap value at the type level.
        let body: Vec<u8> = (0u8..129).flat_map(|i| [i; 32]).collect();
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/blobs/v2")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // 129 hashes exceeds SszList capacity=128; rejected at decode (400) or handler
        // guard (413).
        assert!(resp.status() == 400 || resp.status() == 413);
    }
}

mod blobs_v4_tests {
    use super::test_helpers::auth_token;
    use bytes::Bytes;
    use ethrex_rpc::engine_rest::router;
    use ethrex_rpc::engine_rest::types::blobs::BlobsRequestV4;
    use ethrex_rpc::test_utils::{default_context_with_storage, setup_store};
    use libssz::SszEncode;
    use libssz_types::SszBitvector;
    use tower::ServiceExt;

    async fn build_app() -> (axum::Router, Bytes) {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        (router(ctx), secret)
    }

    // The mempool stores no per-cell data, so the EL cannot serve /blobs/v4 at
    // all and returns 204 No Content (execution-apis #793 §POST /blobs/v4),
    // regardless of which cell indices the request selects.
    #[tokio::test]
    async fn v4_returns_204_no_content() {
        let (app, secret) = build_app().await;
        let token = auth_token(&secret).await;
        let mut bits = SszBitvector::<128>::new();
        bits.set(0, true).unwrap();
        bits.set(5, true).unwrap();
        bits.set(32, true).unwrap();
        let req_body = BlobsRequestV4 {
            versioned_hashes: vec![[0xFFu8; 32]].try_into().unwrap(),
            indices_bitarray: bits,
        };
        let body = req_body.to_ssz();
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/blobs/v4")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 204);
    }

    #[tokio::test]
    async fn v4_empty_bitarray_returns_204() {
        let (app, secret) = build_app().await;
        let token = auth_token(&secret).await;
        let req_body = BlobsRequestV4 {
            versioned_hashes: vec![[0xFFu8; 32]].try_into().unwrap(),
            indices_bitarray: SszBitvector::<128>::new(),
        };
        let body = req_body.to_ssz();
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/blobs/v4")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 204);
    }
}

mod sp3_smoke_tests {
    use super::test_helpers::auth_token;
    use axum::http::StatusCode;
    use bytes::Bytes;
    use ethrex_rpc::engine_rest::router;
    use ethrex_rpc::test_utils::{default_context_with_storage, setup_store};
    use tower::ServiceExt;

    /// Hit every spec-defined endpoint with an auth header and check we never
    /// get a 501. Specific status (200/400/404/413/etc.) is endpoint-dependent.
    #[tokio::test]
    async fn no_engine_rest_endpoint_returns_501() {
        let storage = setup_store().await;
        let mut ctx = default_context_with_storage(storage).await;
        let secret = Bytes::from(vec![0xAB; 32]);
        ctx.node_data.jwt_secret = secret.clone();
        let app = router(ctx);
        let token = auth_token(&secret).await;

        let checks: &[(&str, &str)] = &[
            ("GET", "/identity"),
            ("GET", "/capabilities"),
            ("POST", "/cancun/payloads"),
            ("GET", "/cancun/payloads/0x0102030405060708"),
            ("POST", "/cancun/forkchoice"),
            ("POST", "/cancun/bodies/hash"),
            ("GET", "/cancun/bodies?from=1&count=1"),
            ("POST", "/blobs/v1"),
            ("POST", "/blobs/v2"),
            ("POST", "/blobs/v3"),
            ("POST", "/blobs/v4"),
        ];

        for &(method, uri) in checks {
            let app = app.clone();
            let req = axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/octet-stream")
                .body(axum::body::Body::empty())
                .unwrap();
            let resp = app.oneshot(req).await.unwrap();
            assert_ne!(
                resp.status(),
                StatusCode::NOT_IMPLEMENTED,
                "{method} {uri} returned 501 — endpoint still stubbed"
            );
        }
    }
}

mod transport_tests {
    //! Guards that `axum::serve` (used for the authrpc port in `rpc.rs`) serves
    //! HTTP/2 cleartext (h2c). execution-apis #793 mandates HTTP/2, and the only
    //! #793 CL (consensoor) connects with h2c prior-knowledge and will NOT
    //! downgrade to HTTP/1.1 — so without h2c the entire REST/SSZ path silently
    //! falls back to JSON-RPC. h2c is served by the `hyper_util` auto builder
    //! behind `axum::serve`, which needs axum's `http2` feature (see Cargo.toml).
    //! If that feature regresses, the handshake below fails and this test catches it.

    use axum::Router;
    use axum::routing::get;

    #[tokio::test]
    async fn authrpc_serves_h2c_prior_knowledge() {
        let app = Router::new().route("/ping", get(|| async { "pong" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // h2c prior-knowledge: forces cleartext HTTP/2 with no HTTP/1.1 fallback —
        // exactly how consensoor's httpx(http1=False, http2=True) connects.
        let client = reqwest::Client::builder()
            .http2_prior_knowledge()
            .build()
            .unwrap();
        let resp = client
            .get(format!("http://{addr}/ping"))
            .send()
            .await
            .expect("h2c handshake must succeed — is axum's `http2` feature still enabled?");
        assert_eq!(resp.version(), axum::http::Version::HTTP_2);
        assert_eq!(resp.text().await.unwrap(), "pong");
    }
}

// ── forkchoice_update wire-type round-trips (migrated from types/forkchoice_update.rs) ──
mod forkchoice_update_type_tests {
    use ethrex_rpc::engine_rest::types::blobs::CELLS_PER_EXT_BLOB;
    use ethrex_rpc::engine_rest::types::common::{ForkchoiceState, from_optional, to_optional};
    use ethrex_rpc::engine_rest::types::forkchoice_update::*;
    use ethrex_rpc::engine_rest::types::{amsterdam, cancun, paris};
    use libssz::{SszDecode, SszEncode};
    use libssz_types::SszBitvector;

    #[test]
    fn paris_forkchoice_update_roundtrips_none_attrs() {
        let update = ParisForkchoiceUpdate {
            state: ForkchoiceState {
                head_block_hash: [1; 32],
                safe_block_hash: [2; 32],
                finalized_block_hash: [3; 32],
            },
            payload_attributes: to_optional(None),
        };
        let bytes = update.to_ssz();
        let back = ParisForkchoiceUpdate::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, update);
        assert!(from_optional(&back.payload_attributes).is_none());
    }

    #[test]
    fn paris_forkchoice_update_roundtrips_some_attrs() {
        use ethrex_rpc::engine_rest::types::common::Bytes20;
        let update = ParisForkchoiceUpdate {
            state: ForkchoiceState {
                head_block_hash: [0xFF; 32],
                safe_block_hash: [0; 32],
                finalized_block_hash: [0; 32],
            },
            payload_attributes: to_optional(Some(paris::PayloadAttributes {
                timestamp: 1_700_000_001,
                prev_randao: [9; 32],
                suggested_fee_recipient: Bytes20([10; 20]),
            })),
        };
        let bytes = update.to_ssz();
        let back = ParisForkchoiceUpdate::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, update);
        let attrs = from_optional(&back.payload_attributes).unwrap();
        assert_eq!(attrs.timestamp, 1_700_000_001);
    }

    #[test]
    fn cancun_forkchoice_update_roundtrips_some_attrs() {
        use ethrex_rpc::engine_rest::types::common::Bytes20;
        let update = CancunForkchoiceUpdate {
            state: ForkchoiceState {
                head_block_hash: [0xAA; 32],
                safe_block_hash: [0xBB; 32],
                finalized_block_hash: [0xCC; 32],
            },
            payload_attributes: to_optional(Some(cancun::PayloadAttributes {
                timestamp: 9_999,
                prev_randao: [7; 32],
                suggested_fee_recipient: Bytes20([8; 20]),
                withdrawals: vec![].try_into().unwrap(),
                parent_beacon_block_root: [0xDD; 32],
            })),
        };
        let bytes = update.to_ssz();
        let back = CancunForkchoiceUpdate::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, update);
    }

    #[test]
    fn malformed_bytes_returns_error() {
        let result = CancunForkchoiceUpdate::from_ssz_bytes(&[0u8; 10]);
        assert!(result.is_err());
    }

    #[test]
    fn amsterdam_forkchoice_update_roundtrips_with_custody_columns() {
        use ethrex_rpc::engine_rest::types::common::Bytes20;

        let mut custody = SszBitvector::<CELLS_PER_EXT_BLOB>::new();
        custody.set(3, true).unwrap();
        custody.set(127, true).unwrap();

        let update = AmsterdamForkchoiceUpdate {
            state: ForkchoiceState {
                head_block_hash: [0x11; 32],
                safe_block_hash: [0x22; 32],
                finalized_block_hash: [0x33; 32],
            },
            payload_attributes: to_optional(Some(amsterdam::PayloadAttributes {
                timestamp: 1_700_000_123,
                prev_randao: [4; 32],
                suggested_fee_recipient: Bytes20([5; 20]),
                withdrawals: vec![].try_into().unwrap(),
                parent_beacon_block_root: [0xEE; 32],
                slot_number: 9_001,
                target_gas_limit: 30_000_000,
            })),
            custody_columns: to_optional(Some(custody)),
        };
        let bytes = update.to_ssz();
        let back = AmsterdamForkchoiceUpdate::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, update);
        let attrs = from_optional(&back.payload_attributes).unwrap();
        assert_eq!(attrs.slot_number, 9_001);
        assert_eq!(attrs.target_gas_limit, 30_000_000);
        let c = from_optional(&back.custody_columns).unwrap();
        assert_eq!(c.get(3), Some(true));
        assert_eq!(c.get(127), Some(true));
        assert_eq!(c.get(0), Some(false));
    }

    #[test]
    fn amsterdam_forkchoice_update_roundtrips_none() {
        let update = AmsterdamForkchoiceUpdate {
            state: ForkchoiceState {
                head_block_hash: [0; 32],
                safe_block_hash: [0; 32],
                finalized_block_hash: [0; 32],
            },
            payload_attributes: to_optional(None),
            custody_columns: to_optional(None),
        };
        let bytes = update.to_ssz();
        let back = AmsterdamForkchoiceUpdate::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(back, update);
    }
}
