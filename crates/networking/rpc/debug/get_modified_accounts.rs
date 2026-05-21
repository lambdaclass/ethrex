use std::collections::HashSet;

use ethrex_common::{Address, H256};
use ethrex_storage::Store;
use serde_json::Value;

use crate::{
    types::block_identifier::BlockIdentifier,
    RpcApiContext, RpcErr, RpcHandler,
};

pub struct GetModifiedAccountsByNumberRequest {
    start_block: BlockIdentifier,
    end_block: BlockIdentifier,
}

pub struct GetModifiedAccountsByHashRequest {
    start_hash: H256,
    end_hash: H256,
}

impl RpcHandler for GetModifiedAccountsByNumberRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected 2 params, got {}",
                params.len()
            )));
        }
        Ok(GetModifiedAccountsByNumberRequest {
            start_block: BlockIdentifier::parse(params[0].clone(), 0)?,
            end_block: BlockIdentifier::parse(params[1].clone(), 1)?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let start_number = self
            .start_block
            .resolve_block_number(&context.storage)
            .await?
            .ok_or(RpcErr::Internal("Start block not found".to_string()))?;
        let end_number = self
            .end_block
            .resolve_block_number(&context.storage)
            .await?
            .ok_or(RpcErr::Internal("End block not found".to_string()))?;

        if start_number > end_number {
            return Err(RpcErr::BadParams(
                "Start block must be before end block".to_string(),
            ));
        }

        let start_header = context
            .storage
            .get_block_header(start_number)?
            .ok_or(RpcErr::Internal("Start block header not found".to_string()))?;
        let end_header = context
            .storage
            .get_block_header(end_number)?
            .ok_or(RpcErr::Internal("End block header not found".to_string()))?;

        let addresses = diff_state_roots(
            &context.storage,
            start_header.state_root,
            end_header.state_root,
        )?;

        Ok(serde_json::to_value(addresses)?)
    }
}

impl RpcHandler for GetModifiedAccountsByHashRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected 2 params, got {}",
                params.len()
            )));
        }
        Ok(GetModifiedAccountsByHashRequest {
            start_hash: serde_json::from_value(params[0].clone())?,
            end_hash: serde_json::from_value(params[1].clone())?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let start_header = context
            .storage
            .get_block_header_by_hash(self.start_hash)?
            .ok_or(RpcErr::Internal("Start block not found".to_string()))?;
        let end_header = context
            .storage
            .get_block_header_by_hash(self.end_hash)?
            .ok_or(RpcErr::Internal("End block not found".to_string()))?;

        if start_header.number > end_header.number {
            return Err(RpcErr::BadParams(
                "Start block must be before end block".to_string(),
            ));
        }

        let addresses = diff_state_roots(
            &context.storage,
            start_header.state_root,
            end_header.state_root,
        )?;

        Ok(serde_json::to_value(addresses)?)
    }
}

/// Compare two state roots and return the hashed addresses that differ.
/// Note: without a preimage store, we return hashed addresses (H256) rather than
/// original addresses (Address). Geth returns original addresses via preimage lookup.
fn diff_state_roots(
    storage: &Store,
    start_root: H256,
    end_root: H256,
) -> Result<Vec<Address>, RpcErr> {
    if start_root == end_root {
        return Ok(vec![]);
    }

    // Collect all accounts from both state roots and find differences.
    // This is O(n) in total accounts — acceptable for small states but may be
    // slow on mainnet. A trie-diff algorithm would be more efficient.
    let start_accounts: HashSet<(H256, Vec<u8>)> = storage
        .iter_accounts(start_root)
        .map_err(|e| RpcErr::Internal(e.to_string()))?
        .map(|(hash, state)| {
            let encoded = format!("{:?}{:?}{:?}{:?}", state.nonce, state.balance, state.storage_root, state.code_hash);
            (hash, encoded.into_bytes())
        })
        .collect();

    let mut modified = Vec::new();
    let end_iter = storage
        .iter_accounts(end_root)
        .map_err(|e| RpcErr::Internal(e.to_string()))?;

    let mut end_hashes = HashSet::new();
    for (hash, state) in end_iter {
        end_hashes.insert(hash);
        let encoded = format!("{:?}{:?}{:?}{:?}", state.nonce, state.balance, state.storage_root, state.code_hash);
        let key = (hash, encoded.into_bytes());
        if !start_accounts.contains(&key) {
            // Account was modified or created
            modified.push(Address::from(hash));
        }
    }

    // Find accounts that were deleted (in start but not in end)
    for (hash, _) in &start_accounts {
        if !end_hashes.contains(hash) {
            modified.push(Address::from(*hash));
        }
    }

    Ok(modified)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RpcHandler;
    use serde_json::json;

    // --- GetModifiedAccountsByNumberRequest parse tests ---

    #[test]
    fn parse_by_number_valid() {
        let params = Some(vec![json!("0x1"), json!("0xa")]);
        let req = GetModifiedAccountsByNumberRequest::parse(&params).unwrap();
        assert!(matches!(req.start_block, BlockIdentifier::Number(1)));
        assert!(matches!(req.end_block, BlockIdentifier::Number(10)));
    }

    #[test]
    fn parse_by_number_no_params() {
        assert!(GetModifiedAccountsByNumberRequest::parse(&None).is_err());
    }

    #[test]
    fn parse_by_number_wrong_count() {
        let params = Some(vec![json!("0x1")]);
        assert!(GetModifiedAccountsByNumberRequest::parse(&params).is_err());
    }

    // --- GetModifiedAccountsByHashRequest parse tests ---

    #[test]
    fn parse_by_hash_valid() {
        let params = Some(vec![
            json!("0x0000000000000000000000000000000000000000000000000000000000000001"),
            json!("0x0000000000000000000000000000000000000000000000000000000000000002"),
        ]);
        let req = GetModifiedAccountsByHashRequest::parse(&params).unwrap();
        assert_eq!(req.start_hash, H256::from_low_u64_be(1));
        assert_eq!(req.end_hash, H256::from_low_u64_be(2));
    }

    #[test]
    fn parse_by_hash_no_params() {
        assert!(GetModifiedAccountsByHashRequest::parse(&None).is_err());
    }

    #[test]
    fn parse_by_hash_wrong_count() {
        let params = Some(vec![json!("0x01")]);
        assert!(GetModifiedAccountsByHashRequest::parse(&params).is_err());
    }
}
