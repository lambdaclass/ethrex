use crate::utils::RpcErr;
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use bytes::Bytes;
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, TokenData, Validation, decode, encode,
};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize)]
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

/// Generate a jwt token based on the secret key
/// This should be used to perform authenticated requests to a node with known jwt secret
pub fn generate_jwt_token(secret: &Bytes) -> Result<String, jsonwebtoken::errors::Error> {
    let claims = Claims {
        iat: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Failed to measure time")
            .as_secs() as usize,
        id: None,
        clv: None,
    };
    let header = Header::new(Algorithm::HS256);
    let key = EncodingKey::from_secret(secret);
    encode(&header, &claims, &key)
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
        .as_secs() as usize;
    Ok((now as isize - token_data.claims.iat as isize).abs() > 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_jwt_secret() -> Bytes {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let mut secret = [0u8; 32];
        rng.fill(&mut secret);
        Bytes::from(secret.to_vec())
    }

    #[test]
    fn generated_token_is_valid() {
        let jwt_secret = generate_jwt_secret();
        let jwt_token = generate_jwt_token(&jwt_secret).unwrap();
        assert!(validate_jwt_authentication(&jwt_token, &jwt_secret).is_ok())
    }
}
