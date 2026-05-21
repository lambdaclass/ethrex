use std::collections::BTreeMap;

use ethrex_common::{H256, U256};
use serde::Serialize;
use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler};

pub struct AccountRangeRequest {
    block_hash: H256,
    tx_index: usize,
    start: H256,
    max_results: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountRangeResult {
    accounts: BTreeMap<H256, AccountEntry>,
    #[serde(rename = "next")]
    next: H256,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountEntry {
    balance: U256,
    nonce: u64,
    root: H256,
    code_hash: H256,
    /// The original (unhashed) address. Null when preimage is not available.
    key: Option<H256>,
}

impl RpcHandler for AccountRangeRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 4 {
            return Err(RpcErr::BadParams(format!(
                "Expected 4 params, got {}",
                params.len()
            )));
        }
        let block_hash: H256 = serde_json::from_value(params[0].clone())?;
        let tx_index: usize = serde_json::from_value(params[1].clone())?;
        let start: H256 = serde_json::from_value(params[2].clone())?;
        let max_results: usize = serde_json::from_value(params[3].clone())?;
        Ok(AccountRangeRequest {
            block_hash,
            tx_index,
            start,
            max_results,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let header = context
            .storage
            .get_block_header_by_hash(self.block_hash)?
            .ok_or(RpcErr::Internal("Block not found".to_string()))?;

        let _ = self.tx_index; // TODO: re-execute up to tx_index for precise state

        let iter = context
            .storage
            .iter_accounts_from(header.state_root, self.start)
            .map_err(|e| RpcErr::Internal(e.to_string()))?;

        let mut accounts = BTreeMap::new();
        let mut next = H256::zero();

        for (hashed_addr, account_state) in iter {
            if accounts.len() >= self.max_results {
                next = hashed_addr;
                break;
            }
            accounts.insert(
                hashed_addr,
                AccountEntry {
                    balance: account_state.balance,
                    nonce: account_state.nonce,
                    root: account_state.storage_root,
                    code_hash: account_state.code_hash,
                    key: None, // preimage not available
                },
            );
        }

        Ok(serde_json::to_value(AccountRangeResult { accounts, next })?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RpcHandler;
    use serde_json::json;

    #[test]
    fn parse_valid_params() {
        let params = Some(vec![
            json!("0x0000000000000000000000000000000000000000000000000000000000000001"),
            json!(0),
            json!("0x0000000000000000000000000000000000000000000000000000000000000000"),
            json!(64),
        ]);
        let req = AccountRangeRequest::parse(&params).unwrap();
        assert_eq!(req.block_hash, H256::from_low_u64_be(1));
        assert_eq!(req.tx_index, 0);
        assert_eq!(req.start, H256::zero());
        assert_eq!(req.max_results, 64);
    }

    #[test]
    fn parse_no_params() {
        assert!(AccountRangeRequest::parse(&None).is_err());
    }

    #[test]
    fn parse_wrong_param_count() {
        let params = Some(vec![json!("0x01"), json!(0)]);
        assert!(AccountRangeRequest::parse(&params).is_err());
    }

    #[test]
    fn parse_too_many_params() {
        let params = Some(vec![
            json!("0x0000000000000000000000000000000000000000000000000000000000000001"),
            json!(0),
            json!("0x0000000000000000000000000000000000000000000000000000000000000000"),
            json!(64),
            json!("extra"),
        ]);
        assert!(AccountRangeRequest::parse(&params).is_err());
    }
}
