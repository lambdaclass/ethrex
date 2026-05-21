use ethrex_common::H256;
use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler};

pub struct PreimageRequest {
    _hash: H256,
}

impl RpcHandler for PreimageRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams(format!(
                "Expected 1 param, got {}",
                params.len()
            )));
        }
        let hash: H256 = serde_json::from_value(params[0].clone())?;
        Ok(PreimageRequest { _hash: hash })
    }

    async fn handle(&self, _context: RpcApiContext) -> Result<Value, RpcErr> {
        // ethrex does not maintain a keccak preimage store.
        // Return null to indicate the preimage is not available.
        Ok(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RpcHandler;
    use serde_json::json;

    #[test]
    fn parse_valid_hash() {
        let params = Some(vec![json!(
            "0x0000000000000000000000000000000000000000000000000000000000000001"
        )]);
        let req = PreimageRequest::parse(&params).unwrap();
        assert_eq!(req._hash, H256::from_low_u64_be(1));
    }

    #[test]
    fn parse_no_params() {
        assert!(PreimageRequest::parse(&None).is_err());
    }

    #[test]
    fn parse_too_many_params() {
        let params = Some(vec![json!("0x01"), json!("0x02")]);
        assert!(PreimageRequest::parse(&params).is_err());
    }

    #[tokio::test]
    async fn handle_returns_null() {
        let req = PreimageRequest {
            _hash: H256::zero(),
        };
        let storage = crate::test_utils::setup_store().await;
        let context = crate::test_utils::default_context_with_storage(storage).await;
        let result = req.handle(context).await.unwrap();
        assert_eq!(result, Value::Null);
    }
}
