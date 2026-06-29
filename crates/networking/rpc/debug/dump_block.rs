use std::collections::BTreeMap;

use ethrex_common::{H256, U256};
use ethrex_storage::error::StoreError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler, types::block_identifier::BlockIdentifier};

/// Default number of accounts emitted by `debug_dumpBlock` when the caller does
/// not specify `maxResults`. Callers wanting more can paginate via the `start`
/// cursor in the config object.
const DEFAULT_MAX_RESULTS: usize = 100_000;

/// Hard ceiling on `maxResults` — callers cannot exceed this even via an
/// explicit parameter. Picked to bound response size on large states; geth's
/// `debug_dumpBlock` is unbounded, ethrex diverges here to protect the RPC
/// server from unbounded in-memory responses.
const MAX_RESULTS_CEILING: usize = 100_000;

/// `debug_dumpBlock` — geth-compatible state-trie dump at a given block.
///
/// Response shape:
/// ```text
/// {
///   "root":     <state-root>,
///   "accounts": { <keccak(address)>: { balance, nonce, root, codeHash }, ... },
///   "next":     <keccak(address)>   // present only if more accounts remain
/// }
/// ```
///
/// Divergences from geth:
/// - Emits hashed keys only; addresses and trie preimages are not exposed.
/// - `code` and `storage` are not included. Use `eth_getCode` /
///   `eth_getStorageAt` for those.
/// - `balance` is serialized as a hex string (`"0x..."`) whereas geth uses a
///   decimal string. Callers parsing the balance field should handle both.
/// - `maxResults` is clamped to [`MAX_RESULTS_CEILING`] even if the caller
///   provides a larger value, to protect the RPC server from unbounded
///   responses. Pass `{"start": ..., "maxResults": ...}` to page through
///   larger states.
///
/// Iteration caveat: `Store::iter_accounts_from` silently stops on a node
/// decode failure (see `crates/storage/store.rs`). A truncated response with
/// `next` absent is therefore ambiguous with a complete dump — a fix at the
/// storage layer is needed to distinguish them.
pub struct DumpBlockRequest {
    block: BlockIdentifier,
    config: DumpConfig,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DumpConfig {
    /// Inclusive starting hashed address. Defaults to the start of the trie.
    #[serde(default)]
    start: Option<H256>,
    /// Maximum accounts to emit. Defaults to [`DEFAULT_MAX_RESULTS`].
    #[serde(default)]
    max_results: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DumpResult {
    root: H256,
    accounts: BTreeMap<H256, DumpAccount>,
    /// Set to the next hashed address to query when iteration was truncated by
    /// `maxResults`. Pass it as `start` in a follow-up call to continue.
    #[serde(skip_serializing_if = "Option::is_none")]
    next: Option<H256>,
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
        if params.is_empty() || params.len() > 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected 1 or 2 params, got {}",
                params.len()
            )));
        }
        let block = BlockIdentifier::parse(params[0].clone(), 0)?;
        let config = if params.len() == 2 {
            serde_json::from_value(params[1].clone())?
        } else {
            DumpConfig::default()
        };
        Ok(DumpBlockRequest { block, config })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let block_number = self
            .block
            .resolve_block_number(&context.storage)
            .await?
            .ok_or(RpcErr::WrongParam("Block not found".to_string()))?;
        let header = context
            .storage
            .get_block_header(block_number)?
            .ok_or(RpcErr::WrongParam("Block header not found".to_string()))?;

        let state_root = header.state_root;
        let start = self.config.start.unwrap_or_else(H256::zero);
        let max_results = self
            .config
            .max_results
            .unwrap_or(DEFAULT_MAX_RESULTS)
            .min(MAX_RESULTS_CEILING);
        let storage = context.storage.clone();

        // The trie iterator opens long-lived locked DB transactions and walks
        // the state synchronously — must run off the async runtime.
        let (accounts, next) = tokio::task::spawn_blocking(move || {
            let iter = storage.iter_accounts_from(state_root, start)?;
            let mut accounts: BTreeMap<H256, DumpAccount> = BTreeMap::new();
            let mut next = None;
            for (hashed_addr, account_state) in iter {
                if accounts.len() >= max_results {
                    next = Some(hashed_addr);
                    break;
                }
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
            Ok::<_, StoreError>((accounts, next))
        })
        .await
        .map_err(|e| RpcErr::Internal(format!("dumpBlock task failed: {e}")))??;

        Ok(serde_json::to_value(DumpResult {
            root: state_root,
            accounts,
            next,
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
        assert!(req.config.start.is_none());
        assert!(req.config.max_results.is_none());
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
    fn parse_with_config() {
        let params = Some(vec![
            json!("0xa"),
            json!({
                "start": "0x0000000000000000000000000000000000000000000000000000000000000005",
                "maxResults": 42_u64,
            }),
        ]);
        let req = DumpBlockRequest::parse(&params).unwrap();
        assert_eq!(req.config.start, Some(H256::from_low_u64_be(5)));
        assert_eq!(req.config.max_results, Some(42));
    }

    #[test]
    fn parse_no_params() {
        assert!(DumpBlockRequest::parse(&None).is_err());
    }

    #[test]
    fn parse_too_many_params() {
        let params = Some(vec![json!("0x1"), json!({}), json!("extra")]);
        assert!(DumpBlockRequest::parse(&params).is_err());
    }

    #[test]
    fn parse_empty_params() {
        let params: Option<Vec<Value>> = Some(vec![]);
        assert!(DumpBlockRequest::parse(&params).is_err());
    }
}
