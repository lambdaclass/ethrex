//! `engine_getInclusionListV1` handler. Per
//! `openspec/changes/eip-7805-focil-execution-layer/specs/engine-api-inclusion-list/spec.md`:
//!
//! - Single param: `parentHash` (32-byte hex).
//! - Returns: array of EIP-2718 RLP-encoded transactions, hex-encoded.
//! - Total RLP byte length ≤ `MAX_BYTES_PER_INCLUSION_LIST` (8192).
//! - Excludes blob (EIP-4844) transactions.
//! - 1-second timeout.
//! - Unknown parent → JSON-RPC error code `-38007`.

use std::time::Duration;

use bytes::Bytes;
use ethrex_blockchain::inclusion_list_builder::{
    AccountStateView, IlStateProvider, IlStateProviderError, InclusionListBuilder,
};
use ethrex_common::types::MAX_BYTES_PER_INCLUSION_LIST;
use ethrex_common::{Address, H256};
use ethrex_storage::Store;
use serde_json::Value;
use tracing::debug;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

/// 1-second deadline mandated by the FOCIL execution-apis spec.
const GET_INCLUSION_LIST_V1_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct GetInclusionListV1Request {
    pub parent_hash: H256,
}

/// Adapter from `Store` (keyed by state root) to the IL builder/validator's
/// narrow `IlStateProvider` trait.
struct StoreIlStateProvider<'a> {
    store: &'a Store,
    state_root: H256,
}

impl<'a> IlStateProvider for StoreIlStateProvider<'a> {
    fn get_account(
        &self,
        address: Address,
    ) -> Result<Option<AccountStateView>, IlStateProviderError> {
        let acct = self
            .store
            .get_account_state_by_root(self.state_root, address)
            .map_err(|e| IlStateProviderError::Read(e.to_string()))?;
        Ok(acct.map(|a| AccountStateView {
            nonce: a.nonce,
            balance: a.balance,
        }))
    }
}

impl RpcHandler for GetInclusionListV1Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
        }
        let parent_hash: H256 = serde_json::from_value(params[0].clone())
            .map_err(|_| RpcErr::WrongParam("parentHash".to_string()))?;
        Ok(Self { parent_hash })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(parent_hash = ?self.parent_hash, "engine_getInclusionListV1");
        let parent_hash = self.parent_hash;

        // Build the IL inside `tokio::time::timeout`. The builder is sync but
        // wrapping it in spawn_blocking would be heavier than necessary; the
        // 1-second deadline is generous for the in-memory work the builder
        // performs.
        let result = tokio::time::timeout(GET_INCLUSION_LIST_V1_TIMEOUT, async move {
            // Resolve parent header → state_root + base_fee.
            let parent_header = context
                .storage
                .get_block_header_by_hash(parent_hash)
                .map_err(|e| RpcErr::Internal(e.to_string()))?
                .ok_or_else(|| RpcErr::UnknownParent(format!("0x{:x}", parent_hash)))?;

            let state = StoreIlStateProvider {
                store: &context.storage,
                state_root: parent_header.state_root,
            };
            let base_fee = parent_header.base_fee_per_gas.unwrap_or(0);

            // Read CLI-driven config from context. Hard-cap at 8192 in
            // non-test builds per spec — operators cannot raise this above
            // the protocol limit.
            let cfg = &context.il_config;
            let max_bytes = if cfg!(test) {
                cfg.max_bytes
            } else {
                cfg.max_bytes.min(MAX_BYTES_PER_INCLUSION_LIST)
            };
            let builder = InclusionListBuilder::new(cfg.policy, cfg.per_sender_cap, max_bytes);
            let txs = builder.build(&context.blockchain.mempool, base_fee, &state);

            // Serialize each tx to its EIP-2718 canonical encoding (RLP for
            // legacy, type-prefixed for typed) wrapped as JSON hex strings.
            let encoded: Vec<Bytes> = txs
                .iter()
                .map(|tx| Bytes::from(tx.encode_canonical_to_vec()))
                .collect();

            // Defense in depth: re-check the spec byte cap (8192). Operator
            // overrides via `--il-max-bytes` are clamped above to the spec
            // limit; this catches any future refactor that bypasses that.
            let total: usize = encoded.iter().map(|b| b.len()).sum();
            if total > MAX_BYTES_PER_INCLUSION_LIST {
                return Err(RpcErr::Internal(format!(
                    "inclusion list builder produced {total} bytes, exceeding 8192-byte cap"
                )));
            }

            Ok::<_, RpcErr>(encoded)
        })
        .await;

        let encoded = match result {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(RpcErr::Internal(
                    "engine_getInclusionListV1 timed out".to_string(),
                ));
            }
        };

        // Serialize as `["0x...", "0x..."]`.
        let hex: Vec<String> = encoded
            .iter()
            .map(|b| format!("0x{}", hex::encode(b)))
            .collect();
        serde_json::to_value(hex).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::default_context_with_storage;
    use ethrex_storage::{EngineType, Store};
    use serde_json::json;

    #[tokio::test]
    async fn unknown_parent_returns_38007() {
        let storage = Store::new("", EngineType::InMemory).expect("in-memory store");
        let context = default_context_with_storage(storage).await;

        let req = GetInclusionListV1Request {
            parent_hash: H256::from_low_u64_be(0xdeadbeef),
        };

        let err = req.handle(context).await.expect_err("must fail");
        let metadata: crate::utils::RpcErrorMetadata = err.into();
        assert_eq!(metadata.code, -38007);
        assert_eq!(metadata.message, "Unknown parent");
    }

    #[tokio::test]
    async fn parse_rejects_wrong_param_count() {
        let no_params = GetInclusionListV1Request::parse(&Some(vec![]));
        assert!(matches!(no_params, Err(RpcErr::BadParams(_))));

        let two_params =
            GetInclusionListV1Request::parse(&Some(vec![json!("0x00"), json!("0x01")]));
        assert!(matches!(two_params, Err(RpcErr::BadParams(_))));
    }

    #[tokio::test]
    async fn parse_accepts_valid_hash() {
        let parsed =
            GetInclusionListV1Request::parse(&Some(vec![json!(format!("0x{:064x}", 0x42u64))]))
                .expect("valid 32-byte hex parses");
        assert_eq!(parsed.parent_hash, H256::from_low_u64_be(0x42));
    }
}
