use std::collections::BTreeMap;

use ethrex_common::{H256, U256};
use ethrex_storage::Store;
use ethrex_storage::error::StoreError;
use serde::Serialize;
use serde_json::Value;

use tracing::warn;

use crate::{RpcApiContext, RpcErr, RpcHandler, types::block_identifier::BlockIdentifierOrHash};

/// `debug_accountRange` — paginated iteration over the state trie at a block,
/// matching the shape documented at
/// https://geth.ethereum.org/docs/interacting-with-geth/rpc/ns-debug#debug-accountrange.
///
/// Params: `[blockNrOrHash, txIndex, start, maxResults]`. The first parameter
/// accepts a block number, a tag (`latest` / `earliest` / etc.), a 32-byte
/// hash, or the EIP-1898 object form `{"blockHash": ...}` /
/// `{"blockNumber": ...}`.
///
/// **Known divergences from geth**:
///
/// - `txIndex` is currently ignored — the response is always the state at the
///   *end* of the given block (equivalent to geth's `txIndex == -1`). Returning
///   the state immediately after the Nth transaction requires rebuilding the
///   state trie root from an in-memory VM cache, which is a deferred feature.
///   `code` / `storage` of the accounts are also not emitted; use `eth_getCode`
///   / `eth_getStorageAt` for those. Consequently geth's `nocode`, `nostorage`,
///   and `incompletes` booleans (params 5–7) are not implemented and not
///   accepted — passing more than four parameters errors.
/// - Account entries always have `key: null` because ethrex has no preimage
///   store; callers wanting the original address must hash it themselves to
///   look up by hashed address.
///
/// The trie traversal is wrapped in `tokio::task::spawn_blocking` because
/// `iter_accounts_from` opens long-lived locked DB transactions and performs
/// synchronous trie I/O.
pub struct AccountRangeRequest {
    block: BlockIdentifierOrHash,
    /// Currently ignored — see type-level doc.
    tx_index: i64,
    start: H256,
    max_results: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountRangeResult {
    accounts: BTreeMap<H256, AccountEntry>,
    /// First hashed address NOT included in this page. All zeroes means the
    /// iteration is complete (matches geth's sentinel).
    next: H256,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountEntry {
    balance: U256,
    nonce: u64,
    root: H256,
    code_hash: H256,
    /// Original (unhashed) address. Always `null` — ethrex has no preimage store.
    #[serde(skip_serializing_if = "Option::is_none")]
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
        let block = BlockIdentifierOrHash::parse(params[0].clone(), 0)?;
        let tx_index: i64 = serde_json::from_value(params[1].clone())
            .map_err(|e| RpcErr::BadParams(format!("invalid txIndex: {e}")))?;
        let start: H256 = serde_json::from_value(params[2].clone())
            .map_err(|e| RpcErr::BadParams(format!("invalid start hash: {e}")))?;
        let max_results: usize = serde_json::from_value(params[3].clone())
            .map_err(|e| RpcErr::BadParams(format!("invalid maxResults: {e}")))?;
        if max_results == 0 {
            return Err(RpcErr::BadParams(
                "maxResults must be greater than 0".to_owned(),
            ));
        }
        Ok(AccountRangeRequest {
            block,
            tx_index,
            start,
            max_results,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let header = self
            .block
            .resolve_block_header(&context.storage)
            .await?
            .ok_or_else(|| RpcErr::WrongParam("Block not found".to_string()))?;

        // `tx_index` is parsed but not honoured yet — we always return end-of-block
        // state. Negative values (geth's `-1` sentinel) and values >= the block's tx
        // count are equivalent, so no error. For non-negative values that would
        // require mid-block state reconstruction we emit a warning.
        if self.tx_index >= 0 {
            warn!(
                tx_index = self.tx_index,
                "debug_accountRange: txIndex is not yet supported, returning end-of-block state"
            );
        }

        let state_root = header.state_root;
        let start = self.start;
        let max_results = self.max_results;
        let storage = context.storage.clone();

        let (accounts, next) = tokio::task::spawn_blocking(move || {
            collect_account_range(&storage, state_root, start, max_results)
        })
        .await
        .map_err(|e| RpcErr::Internal(format!("accountRange task failed: {e}")))??;

        Ok(serde_json::to_value(AccountRangeResult { accounts, next })?)
    }
}

fn collect_account_range(
    storage: &Store,
    state_root: H256,
    start: H256,
    max_results: usize,
) -> Result<(BTreeMap<H256, AccountEntry>, H256), StoreError> {
    let iter = storage.iter_accounts_from(state_root, start)?;
    let mut accounts = BTreeMap::new();
    let mut next = H256::zero();
    for (hashed_addr, account_state) in iter {
        if accounts.len() >= max_results {
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
                key: None,
            },
        );
    }
    Ok((accounts, next))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RpcHandler;
    use crate::types::block_identifier::BlockIdentifier;
    use serde_json::json;

    #[test]
    fn parse_valid_params_by_hash() {
        let params = Some(vec![
            json!("0x0000000000000000000000000000000000000000000000000000000000000001"),
            json!(0),
            json!("0x0000000000000000000000000000000000000000000000000000000000000000"),
            json!(64),
        ]);
        let req = AccountRangeRequest::parse(&params).unwrap();
        assert!(matches!(req.block, BlockIdentifierOrHash::Hash(_)));
        assert_eq!(req.tx_index, 0);
        assert_eq!(req.start, H256::zero());
        assert_eq!(req.max_results, 64);
    }

    #[test]
    fn parse_valid_params_by_number() {
        let params = Some(vec![
            json!("0xa"),
            json!(-1),
            json!("0x0000000000000000000000000000000000000000000000000000000000000000"),
            json!(64),
        ]);
        let req = AccountRangeRequest::parse(&params).unwrap();
        assert!(matches!(
            req.block,
            BlockIdentifierOrHash::Identifier(BlockIdentifier::Number(10))
        ));
        assert_eq!(req.tx_index, -1);
    }

    #[test]
    fn parse_valid_params_by_tag() {
        let params = Some(vec![
            json!("latest"),
            json!(0),
            json!("0x0000000000000000000000000000000000000000000000000000000000000000"),
            json!(64),
        ]);
        let req = AccountRangeRequest::parse(&params).unwrap();
        assert!(matches!(req.block, BlockIdentifierOrHash::Identifier(_)));
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
