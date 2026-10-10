use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
    typed_header::TypedHeaderRejection,
};
use bytes::Bytes;
use jsonwebtoken::{Algorithm, DecodingKey, TokenData, Validation, decode};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub enum AuthenticationError {
    InvalidIssuedAtClaim,
    TokenDecodingError,
    MissingAuthentication,
}

impl AuthenticationError {
    /// Plain-text reason sent with the 401, worded like geth's.
    fn reason(&self) -> &'static str {
        match self {
            AuthenticationError::MissingAuthentication => "missing token",
            AuthenticationError::TokenDecodingError => "invalid token",
            AuthenticationError::InvalidIssuedAtClaim => "stale or future token",
        }
    }
}

/// Auth-RPC router middleware: rejects a request without a valid JWT before its body is
/// read. Like geth and reth, the rejection is an HTTP 401 with a plain-text reason, not a
/// JSON-RPC error, since no request has been parsed to take an id from.
pub(crate) async fn require_jwt(
    State(secret): State<Bytes>,
    auth_header: Result<TypedHeader<Authorization<Bearer>>, TypedHeaderRejection>,
    request: Request,
    next: Next,
) -> Response {
    // A malformed `Authorization` header is no better than a missing one.
    let result = match auth_header {
        Ok(TypedHeader(header)) => validate_jwt_authentication(header.token(), &secret),
        Err(_) => Err(AuthenticationError::MissingAuthentication),
    };
    match result {
        Ok(()) => next.run(request).await,
        Err(error) => {
            // Without this, a wrong secret only shows up on this side as the consensus
            // layer having gone silent.
            warn!(
                "Rejected Auth-RPC request: {}. Check that the consensus client uses this node's JWT secret and that both clocks are in sync",
                error.reason()
            );
            (StatusCode::UNAUTHORIZED, error.reason()).into_response()
        }
    }
}

// JWT claims struct
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iat: usize,
    id: Option<String>,
    clv: Option<String>,
}

/// Authenticates bearer jwt to check that authrpc calls are sent by the consensus layer
pub fn validate_jwt_authentication(token: &str, secret: &Bytes) -> Result<(), AuthenticationError> {
    let decoding_key = DecodingKey::from_secret(secret);
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = false;
    validation.set_required_spec_claims(&["iat"]);
    match decode::<Claims>(token, &decoding_key, &validation) {
        Ok(token_data) => {
            if invalid_issued_at_claim(token_data)? {
                Err(AuthenticationError::InvalidIssuedAtClaim)
            } else {
                Ok(())
            }
        }
        Err(_) => Err(AuthenticationError::TokenDecodingError),
    }
}

/// Checks that the "iat" timestamp in the claim is less than 60 seconds from now
fn invalid_issued_at_claim(token_data: TokenData<Claims>) -> Result<bool, AuthenticationError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuthenticationError::InvalidIssuedAtClaim)?
        .as_secs();
    Ok((now as i64 - token_data.claims.iat as i64).abs() > 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Debug, Serialize, Deserialize)]
    struct FaultyClaims {
        id: Option<String>,
        clv: Option<String>,
    }

    #[test]
    fn test_iat_missing_fails() {
        // Our `Claims` type expect `iat` so JWTs with it would simply fail to deserialize.
        let secret = Bytes::from("my_secret_key");
        let faulty_claims = FaultyClaims {
            id: None,
            clv: None,
        };
        let token = encode(
            &Header::default(),
            &faulty_claims,
            &EncodingKey::from_secret(&secret),
        )
        .unwrap();

        let res = validate_jwt_authentication(&token, &secret);
        assert_eq!(res.unwrap_err(), AuthenticationError::TokenDecodingError);
    }

    #[test]
    fn test_iat_zero_fails() {
        let secret = Bytes::from("my_secret_key");
        let faulty_claims = Claims {
            iat: 0,
            id: None,
            clv: None,
        };
        let token = encode(
            &Header::default(),
            &faulty_claims,
            &EncodingKey::from_secret(&secret),
        )
        .unwrap();
        let res = validate_jwt_authentication(&token, &secret);
        assert_eq!(res.unwrap_err(), AuthenticationError::InvalidIssuedAtClaim);
    }

    #[test]
    fn test_iat_too_old_fails() {
        let secret = Bytes::from("my_secret_key");
        let old_iat = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 120;
        let faulty_claims = Claims {
            iat: old_iat as usize,
            id: None,
            clv: None,
        };
        let token = encode(
            &Header::default(),
            &faulty_claims,
            &EncodingKey::from_secret(&secret),
        )
        .unwrap();
        let res = validate_jwt_authentication(&token, &secret);
        assert_eq!(res.unwrap_err(), AuthenticationError::InvalidIssuedAtClaim);
    }

    #[test]
    fn test_iat_future_fails() {
        let secret = Bytes::from("my_secret_key");
        let future_iat = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 120;
        let faulty_claims = Claims {
            iat: future_iat as usize,
            id: None,
            clv: None,
        };
        let token = encode(
            &Header::default(),
            &faulty_claims,
            &EncodingKey::from_secret(&secret),
        )
        .unwrap();
        let res = validate_jwt_authentication(&token, &secret);
        assert_eq!(res.unwrap_err(), AuthenticationError::InvalidIssuedAtClaim);
    }

    #[test]
    fn test_iat_within_range_passes() {
        let secret = Bytes::from("my_secret_key");

        // Test with iat 59 seconds in the past
        let valid_iat = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 59;

        let valid_claims = Claims {
            iat: valid_iat as usize,
            id: None,
            clv: None,
        };
        let token = encode(
            &Header::default(),
            &valid_claims,
            &EncodingKey::from_secret(&secret),
        )
        .unwrap();
        let res = validate_jwt_authentication(&token, &secret);
        assert!(res.is_ok());

        // Test with iat 59 seconds in the future
        let valid_iat_future = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 59;
        let valid_claims_future = Claims {
            iat: valid_iat_future as usize,
            id: None,
            clv: None,
        };
        let token_future = encode(
            &Header::default(),
            &valid_claims_future,
            &EncodingKey::from_secret(&secret),
        )
        .unwrap();
        let res_future = validate_jwt_authentication(&token_future, &secret);
        assert!(res_future.is_ok());
    }
}
