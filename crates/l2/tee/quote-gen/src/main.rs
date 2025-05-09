use std::collections::HashMap;
use std::sync::Arc;

use configfs_tsm::create_tdx_quote;
use zerocopy::IntoBytes;

use eth_encode_packed::abi::encode_packed;
use eth_encode_packed::ethabi::ethereum_types::U256;
use eth_encode_packed::{SolidityDataType, TakeLastXBytes};

use alloy::signers::{local::PrivateKeySigner, Signer};

use serde::Serialize;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};

struct AppState {
    signer: PrivateKeySigner,
}

fn inc(n: u64) -> u64 {
    n + 1
}

fn run_inc(input: u64) -> (u64, Vec<u8>) {
    let output = inc(input);
    let data = vec![
        SolidityDataType::NumberWithShift(U256::from(input), TakeLastXBytes(64)),
        SolidityDataType::NumberWithShift(U256::from(output), TakeLastXBytes(64)),
    ];
    let (bytes, _) = encode_packed(&data);
    (output, bytes)
}

type GenericError = Result<(), Box<dyn std::error::Error>>;

#[derive(Serialize)]
struct GetKeyResponse {
    address: String,
    quote: String,
}

async fn handle_getkey(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GetKeyResponse>, StatusCode> {
    let mut digest_slice = [0u8; 64];
    digest_slice
        .split_at_mut(20)
        .0
        .copy_from_slice(state.signer.address().as_bytes());
    let quote = create_tdx_quote(digest_slice).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let quote = hex::encode(quote);

    Ok(Json(GetKeyResponse {
        address: state.signer.address().to_string(),
        quote,
    }))
}

#[derive(Serialize)]
struct TransitionResponse {
    new_state: u64,
    signature: String,
}

async fn handle_transition(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<TransitionResponse>, StatusCode> {
    let current = params.get("state").ok_or(StatusCode::BAD_REQUEST)?;
    let current = current.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    let (new_state, bound_data) = run_inc(current);
    let signature = state
        .signer
        .sign_message(&bound_data)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(TransitionResponse {
        new_state: new_state,
        signature: signature.to_string(),
    }))
}

#[tokio::main]
async fn main() -> GenericError {
    let state = Arc::new(AppState {
        signer: PrivateKeySigner::random(),
    });

    let app = Router::new()
        .route("/getkey", get(handle_getkey))
        .route("/transition", get(handle_transition))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3001").await?;
    axum::serve(listener, app).await?;
    Ok(())
}
