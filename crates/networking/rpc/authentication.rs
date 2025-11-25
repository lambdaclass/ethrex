use crate::utils::RpcErr;
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use bytes::Bytes;
use jsonwebtoken::{Algorithm, DecodingKey, TokenData, Validation, decode};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize, PartialEq)]
pub enum AuthenticationError {
    InvalidIssuedAtClaim,
    TokenDecodingError,
    MissingAuthentication,
}

pub fn authenticate(
    secret: &Bytes,
    auth_header: Option<TypedHeader<Authorization<Bearer>>>,
) -> Result<(), RpcErr> {
    match auth_header {
        Some(TypedHeader(auth_header)) => {
            let token = auth_header.token();
            validate_jwt_authentication(token, secret).map_err(RpcErr::AuthenticationError)
        }
        None => Err(RpcErr::AuthenticationError(
            AuthenticationError::MissingAuthentication,
        )),
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
