//! `eth_capabilities` (execution-apis #755).

use ethrex_common::{H256, constants::EMPTY_TRIE_HASH, types::BlockHeader};
use ethrex_storage::{Store, error::StoreError};
use serde_json::{Value, json, value::RawValue};
use tracing::debug;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

pub struct CapabilitiesRequest;

impl RpcHandler for CapabilitiesRequest {
    fn parse(_params: &Option<Vec<Box<RawValue>>>) -> Result<Self, RpcErr> {
        Ok(Self)
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("Requested capabilities");
        let storage = &context.storage;
        let head_hash = storage
            .get_latest_canonical_block_hash()
            .await?
            .ok_or_else(|| RpcErr::Internal("no canonical head".to_string()))?;
        let head = storage
            .get_block_header_by_hash(head_hash)?
            .ok_or_else(|| RpcErr::Internal("canonical head header is missing".to_string()))?;
        let head_number = head.number;

        // Bodies are stored from `earliest` to the head and never pruned.
        let earliest = storage.get_earliest_block_number().await?;
        let tx = earliest.max(storage.get_tx_index_floor().await?.unwrap_or_default());
        let logs = oldest_block_with_receipts(storage, earliest).await?;
        // A database from before these bounds were tracked may still have `earliest` at
        // genesis and no floor. They are corrected once the node is synced, and until
        // then no history is reported from them.
        let reconciled = storage.history_bounds_reconciled().await?;
        let history = |oldest_block: u64| {
            if reconciled {
                available_from(oldest_block, head_number)
            } else {
                json!({"disabled": true})
            }
        };

        // A block count: forkchoice keeps the state of the blocks down to `head - retention`.
        let retention = storage.reorg_retention()?;
        let state_storage = storage.clone();
        let oldest_state = tokio::task::spawn_blocking(move || {
            oldest_block_with_state(&state_storage, &head, retention)
        })
        .await
        .map_err(|error| RpcErr::Internal(error.to_string()))??;
        let state = match oldest_state {
            Some(oldest) => json!({
                "disabled": false,
                "oldestBlock": format!("{oldest:#x}"),
                "deleteStrategy": {"type": "window", "retentionBlocks": format!("{retention:#x}")},
            }),
            None => json!({"disabled": true}),
        };

        Ok(json!({
            "head": {"number": format!("{head_number:#x}"), "hash": head_hash},
            "state": state,
            "tx": history(tx),
            "logs": history(logs),
            // `eth_getTransactionReceipt` needs the tx index, so this stays at `tx` even
            // where `eth_getBlockReceipts` serves from `logs`.
            "receipts": history(logs.max(tx)),
            "blocks": history(earliest),
            "stateproofs": state,
        }))
    }
}

/// Disabled above the head: a snap pivot that is still the head has no receipts.
fn available_from(oldest_block: u64, head: u64) -> Value {
    if oldest_block > head {
        return json!({"disabled": true});
    }
    json!({"disabled": false, "oldestBlock": format!("{oldest_block:#x}")})
}

/// A snap pivot's body is stored but its receipts only arrive with history backfill.
/// Every other block from `earliest` up was executed or backfilled, which stores receipts.
async fn oldest_block_with_receipts(storage: &Store, earliest: u64) -> Result<u64, RpcErr> {
    if storage.get_receipt(earliest, 0).await?.is_some() {
        return Ok(earliest);
    }
    match storage.get_block_header(earliest)? {
        Some(header) if header.transactions_root != *EMPTY_TRIE_HASH => {
            Ok(earliest.saturating_add(1))
        }
        _ => Ok(earliest),
    }
}

/// Lowest block, at most `retention` below the head, whose state is held; `None` while syncing.
fn oldest_block_with_state(
    storage: &Store,
    head: &BlockHeader,
    retention: u64,
) -> Result<Option<u64>, RpcErr> {
    if !holds_state(storage, head.state_root)? {
        return Ok(None);
    }
    // Follow parent hashes so a concurrent reorg cannot move the walk to another fork.
    let mut oldest = head.clone();
    while oldest.number > 0 && head.number - oldest.number < retention {
        let Some(parent) = storage.get_block_header_by_hash(oldest.parent_hash)? else {
            break;
        };
        // Guards against a corrupt parent link looping or passing the head.
        if parent.number.checked_add(1) != Some(oldest.number) {
            break;
        }
        if !holds_state(storage, parent.state_root)? {
            break;
        }
        oldest = parent;
    }
    Ok(Some(oldest.number))
}

/// Recent states are layer-cache hits, which spares opening a trie for each.
fn holds_state(storage: &Store, state_root: H256) -> Result<bool, StoreError> {
    Ok(storage.is_state_in_layer_cache(state_root)? || storage.has_state_root(state_root)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{add_legacy_tx_blocks, context_without_p2p, setup_store};
    use ethrex_common::types::{Block, BlockBody, Receipt, TxType};
    use ethrex_storage::{BackfilledBlock, EngineType};

    async fn capabilities(context: &RpcApiContext) -> Value {
        let request: crate::utils::RpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_capabilities",
            "params": [],
        }))
        .unwrap();
        crate::rpc::map_http_requests(&request, context.clone())
            .await
            .unwrap()
    }

    async fn add_block_with_state_root(storage: &Store, state_root: H256) -> BlockHeader {
        let parent = storage
            .get_block_header(storage.get_latest_block_number().unwrap())
            .unwrap()
            .unwrap();
        let header = BlockHeader {
            hash: Default::default(),
            number: parent.number + 1,
            parent_hash: parent.hash(),
            state_root,
            ..parent
        };
        let hash = header.hash();
        storage
            .add_block(Block::new(header.clone(), BlockBody::default()))
            .await
            .unwrap();
        storage
            .forkchoice_update(vec![], header.number, hash, None, None)
            .await
            .unwrap();
        header
    }

    fn receipt() -> Receipt {
        Receipt {
            tx_type: TxType::Legacy,
            succeeded: true,
            cumulative_gas_used: 21_000,
            logs: vec![],
            payer: None,
            frame_receipts: None,
        }
    }

    #[tokio::test]
    async fn genesis_node_serves_everything_from_genesis() {
        let storage = setup_store().await;
        let genesis = storage.get_block_header(0).unwrap().unwrap();
        let context = context_without_p2p(storage);
        let from_genesis = json!({"disabled": false, "oldestBlock": "0x0"});
        let state = json!({
            "disabled": false,
            "oldestBlock": "0x0",
            "deleteStrategy": {"type": "window", "retentionBlocks": "0x2710"},
        });
        assert_eq!(
            capabilities(&context).await,
            json!({
                "head": {"number": "0x0", "hash": format!("{:#x}", genesis.hash())},
                "state": state,
                "tx": from_genesis,
                "logs": from_genesis,
                "receipts": from_genesis,
                "blocks": from_genesis,
                "stateproofs": state,
            })
        );
    }

    /// A database created before the history bounds were tracked carries no marker.
    #[tokio::test]
    async fn history_is_disabled_until_the_history_bounds_are_reconciled() {
        let storage = Store::new("", EngineType::InMemory).unwrap();
        let genesis = BlockHeader {
            transactions_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let genesis_hash = genesis.hash();
        storage
            .add_block(Block::new(genesis, BlockBody::default()))
            .await
            .unwrap();
        storage.update_earliest_block_number(0).await.unwrap();
        storage
            .forkchoice_update(vec![], 0, genesis_hash, None, None)
            .await
            .unwrap();
        let context = context_without_p2p(storage);
        let history = ["blocks", "tx", "logs", "receipts"];

        let result = capabilities(&context).await;
        assert_eq!(result["head"]["number"], "0x0");
        for resource in history {
            assert_eq!(result[resource], json!({"disabled": true}), "{resource}");
        }

        context
            .storage
            .mark_history_bounds_reconciled(None)
            .await
            .unwrap();
        let result = capabilities(&context).await;
        for resource in history {
            assert_eq!(
                result[resource],
                json!({"disabled": false, "oldestBlock": "0x0"}),
                "{resource}"
            );
        }
    }

    #[tokio::test]
    async fn state_starts_above_the_newest_block_without_state() {
        let storage = setup_store().await;
        let genesis_root = storage.get_block_header(0).unwrap().unwrap().state_root;
        // Block 1's state is not held; blocks 2 and 3 reuse genesis state, which is.
        add_block_with_state_root(&storage, H256::repeat_byte(0xaa)).await;
        add_block_with_state_root(&storage, genesis_root).await;
        let head = add_block_with_state_root(&storage, genesis_root).await;
        let context = context_without_p2p(storage);

        let result = capabilities(&context).await;
        assert_eq!(result["head"]["number"], "0x3");
        assert_eq!(result["head"]["hash"], format!("{:#x}", head.hash()));
        for resource in ["state", "stateproofs"] {
            assert_eq!(result[resource]["oldestBlock"], "0x2", "{resource}");
            assert_eq!(result[resource]["disabled"], false, "{resource}");
        }

        assert_eq!(
            oldest_block_with_state(&context.storage, &head, 1).unwrap(),
            Some(2)
        );
    }

    #[tokio::test]
    async fn state_walk_follows_the_head_fork_after_a_reorg() {
        let storage = setup_store().await;
        let genesis = storage.get_block_header(0).unwrap().unwrap();
        add_block_with_state_root(&storage, genesis.state_root).await;
        let old_head = add_block_with_state_root(&storage, genesis.state_root).await;
        // Reorg to a block 1 whose state is not held.
        let other = BlockHeader {
            hash: Default::default(),
            number: 1,
            parent_hash: genesis.hash(),
            state_root: H256::repeat_byte(0xaa),
            ..genesis
        };
        let other_hash = other.hash();
        storage
            .add_block(Block::new(other, BlockBody::default()))
            .await
            .unwrap();
        storage
            .forkchoice_update(vec![], 1, other_hash, None, None)
            .await
            .unwrap();

        assert_eq!(
            oldest_block_with_state(&storage, &old_head, 10).unwrap(),
            Some(0)
        );
    }

    #[tokio::test]
    async fn state_is_disabled_when_the_head_state_is_not_held() {
        let storage = setup_store().await;
        add_block_with_state_root(&storage, H256::repeat_byte(0xaa)).await;
        let context = context_without_p2p(storage);

        let result = capabilities(&context).await;
        assert_eq!(result["state"], json!({"disabled": true}));
        assert_eq!(result["stateproofs"], json!({"disabled": true}));
        assert_eq!(result["blocks"]["oldestBlock"], "0x0");
    }

    #[tokio::test]
    async fn receipts_and_logs_skip_a_pivot_without_receipts() {
        let storage = setup_store().await;
        add_legacy_tx_blocks(&storage, 2, 1).await;
        storage.update_earliest_block_number(1).await.unwrap();
        let block_two = storage.get_block_header(2).unwrap().unwrap();
        storage
            .add_receipts(block_two.hash(), vec![receipt()])
            .await
            .unwrap();
        let context = context_without_p2p(storage);

        let result = capabilities(&context).await;
        assert_eq!(result["blocks"]["oldestBlock"], "0x1");
        assert_eq!(result["tx"]["oldestBlock"], "0x1");
        assert_eq!(result["receipts"]["oldestBlock"], "0x2");
        assert_eq!(result["logs"]["oldestBlock"], "0x2");

        let block_one = context.storage.get_block_header(1).unwrap().unwrap();
        context
            .storage
            .add_receipts(block_one.hash(), vec![receipt()])
            .await
            .unwrap();
        let result = capabilities(&context).await;
        assert_eq!(result["receipts"]["oldestBlock"], "0x1");
        assert_eq!(result["logs"]["oldestBlock"], "0x1");
    }

    #[tokio::test]
    async fn receipts_and_logs_are_disabled_at_a_pivot_head_without_receipts() {
        let storage = setup_store().await;
        add_legacy_tx_blocks(&storage, 1, 1).await;
        storage.update_earliest_block_number(1).await.unwrap();
        let context = context_without_p2p(storage);

        let result = capabilities(&context).await;
        assert_eq!(result["receipts"], json!({"disabled": true}));
        assert_eq!(result["logs"], json!({"disabled": true}));
        assert_eq!(result["tx"]["oldestBlock"], "0x1");
        assert_eq!(result["blocks"]["oldestBlock"], "0x1");
    }

    #[tokio::test]
    async fn tx_and_receipts_start_at_the_tx_index_floor() {
        let storage = setup_store().await;
        add_legacy_tx_blocks(&storage, 2, 1).await;
        storage.update_earliest_block_number(2).await.unwrap();
        let block_one = storage.get_block_header(1).unwrap().unwrap();
        let body = storage.get_block_body(1).await.unwrap().unwrap();
        storage
            .add_backfilled_blocks(
                vec![BackfilledBlock {
                    header: block_one,
                    body,
                    receipts: vec![receipt()],
                    index_transactions: false,
                }],
                1,
            )
            .await
            .unwrap();
        let context = context_without_p2p(storage);

        let result = capabilities(&context).await;
        assert_eq!(result["blocks"]["oldestBlock"], "0x1");
        assert_eq!(result["logs"]["oldestBlock"], "0x1");
        assert_eq!(result["tx"]["oldestBlock"], "0x2");
        assert_eq!(result["receipts"]["oldestBlock"], "0x2");
    }
}
