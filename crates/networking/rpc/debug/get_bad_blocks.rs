use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler};

pub struct GetBadBlocksRequest;

impl RpcHandler for GetBadBlocksRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        if let Some(params) = params
            && !params.is_empty()
        {
            return Err(RpcErr::BadParams(format!(
                "Expected no params and {} were provided",
                params.len()
            )));
        }
        Ok(GetBadBlocksRequest)
    }

    async fn handle(&self, _context: RpcApiContext) -> Result<Value, RpcErr> {
        // ethrex records rejected-block hashes in INVALID_CHAINS (hash ->
        // latest_valid_hash) but does not persist the full block RLP / body /
        // error reason that geth's debug_getBadBlocks response requires, so we
        // return the spec-valid empty array. A real implementation needs a
        // bad-block metadata table with eviction policy.
        Ok(Value::Array(vec![]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RpcHandler;
    use serde_json::json;

    #[test]
    fn parse_no_params() {
        let result = GetBadBlocksRequest::parse(&None);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_empty_params() {
        let result = GetBadBlocksRequest::parse(&Some(vec![]));
        assert!(result.is_ok());
    }

    #[test]
    fn parse_with_params_fails() {
        let result = GetBadBlocksRequest::parse(&Some(vec![json!("extra")]));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn handle_returns_empty_array() {
        let req = GetBadBlocksRequest;
        let storage = crate::test_utils::setup_store().await;
        let context = crate::test_utils::default_context_with_storage(storage).await;
        let result = req.handle(context).await.unwrap();
        assert_eq!(result, Value::Array(vec![]));
    }
}
