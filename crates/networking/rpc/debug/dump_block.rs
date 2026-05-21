use std::collections::BTreeMap;

use ethrex_common::{H256, U256};
use serde::Serialize;
use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler, types::block_identifier::BlockIdentifier};

pub struct DumpBlockRequest {
    block: BlockIdentifier,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DumpResult {
    root: H256,
    accounts: BTreeMap<H256, DumpAccount>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DumpAccount {
    balance: U256,
    nonce: u64,
    root: H256,
    code_hash: H256,
}

impl RpcHandler for DumpBlockRequest {
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
        Ok(DumpBlockRequest {
            block: BlockIdentifier::parse(params[0].clone(), 0)?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let block_number = self
            .block
            .resolve_block_number(&context.storage)
            .await?
            .ok_or(RpcErr::Internal("Block not found".to_string()))?;
        let header = context
            .storage
            .get_block_header(block_number)?
            .ok_or(RpcErr::Internal("Block header not found".to_string()))?;

        let iter = context
            .storage
            .iter_accounts(header.state_root)
            .map_err(|e| RpcErr::Internal(e.to_string()))?;

        let mut accounts = BTreeMap::new();
        for (hashed_addr, account_state) in iter {
            accounts.insert(
                hashed_addr,
                DumpAccount {
                    balance: account_state.balance,
                    nonce: account_state.nonce,
                    root: account_state.storage_root,
                    code_hash: account_state.code_hash,
                },
            );
        }

        Ok(serde_json::to_value(DumpResult {
            root: header.state_root,
            accounts,
        })?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RpcHandler;
    use serde_json::json;

    #[test]
    fn parse_block_number() {
        let params = Some(vec![json!("0xa")]);
        let req = DumpBlockRequest::parse(&params).unwrap();
        assert!(matches!(req.block, BlockIdentifier::Number(10)));
    }

    #[test]
    fn parse_latest_tag() {
        let params = Some(vec![json!("latest")]);
        let req = DumpBlockRequest::parse(&params).unwrap();
        assert!(matches!(
            req.block,
            BlockIdentifier::Tag(crate::types::block_identifier::BlockTag::Latest)
        ));
    }

    #[test]
    fn parse_no_params() {
        assert!(DumpBlockRequest::parse(&None).is_err());
    }

    #[test]
    fn parse_too_many_params() {
        let params = Some(vec![json!("0x1"), json!("extra")]);
        assert!(DumpBlockRequest::parse(&params).is_err());
    }
}
