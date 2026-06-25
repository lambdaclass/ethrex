// The behaviour of the filtering endpoints is based on:
// - Manually testing the behaviour deploying contracts on the Sepolia test network.
// - Go-Ethereum, specifically: https://github.com/ethereum/go-ethereum/blob/368e16f39d6c7e5cce72a92ec289adbfbaed4854/eth/filters/filter.go
// - Ethereum's reference: https://ethereum.org/en/developers/docs/apis/json-rpc/#eth_newfilter
use crate::{
    rpc::{RpcApiContext, RpcHandler},
    types::{
        block_identifier::{BlockIdentifier, BlockTag},
        receipt::RpcLog,
    },
    utils::RpcErr,
};
use ethereum_types::{Bloom, BloomInput};
use ethrex_common::{H160, H256};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::Store;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum AddressFilter {
    Single(H160),
    Many(Vec<H160>),
}

impl AsRef<[H160]> for AddressFilter {
    fn as_ref(&self) -> &[H160] {
        match self {
            AddressFilter::Single(address) => std::slice::from_ref(address),
            AddressFilter::Many(addresses) => addresses.as_ref(),
        }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum TopicFilter {
    Topic(Option<H256>),
    Topics(Vec<Option<H256>>),
}

#[derive(Debug, Clone)]
pub struct LogsFilter {
    /// The oldest block from which to start
    /// retrieving logs.
    /// Will default to `latest` if not provided.
    pub from_block: BlockIdentifier,
    /// Up to which block to stop retrieving logs.
    /// Will default to `latest` if not provided.
    pub to_block: BlockIdentifier,
    /// The addresses from where the logs origin from.
    pub address_filters: Option<AddressFilter>,
    /// Which topics to filter.
    pub topics: Vec<TopicFilter>,
}
impl RpcHandler for LogsFilter {
    fn parse(params: &Option<Vec<Value>>) -> Result<LogsFilter, RpcErr> {
        match params.as_deref() {
            Some([param]) => {
                let param = param
                    .as_object()
                    .ok_or(RpcErr::BadParams("Param is not a object".to_owned()))?;
                let from_block = param
                    .get("fromBlock")
                    .map(|block_number| BlockIdentifier::parse(block_number.clone(), 0))
                    .transpose()?
                    .unwrap_or(BlockIdentifier::Tag(BlockTag::Latest));
                let to_block = param
                    .get("toBlock")
                    .map(|block_number| BlockIdentifier::parse(block_number.clone(), 0))
                    .transpose()?
                    .unwrap_or(BlockIdentifier::Tag(BlockTag::Latest));
                let address_filters = param
                    .get("address")
                    .map(|address| {
                        match serde_json::from_value::<Option<AddressFilter>>(address.clone()) {
                            Ok(filters) => Ok(filters),
                            _ => Err(RpcErr::WrongParam("address".to_string())),
                        }
                    })
                    .transpose()?
                    .flatten();
                let topics_filters = param
                    .get("topics")
                    .ok_or_else(|| RpcErr::MissingParam("topics".to_string()))
                    .and_then(|topics| {
                        match serde_json::from_value::<Option<Vec<TopicFilter>>>(topics.clone()) {
                            Ok(filters) => Ok(filters),
                            _ => Err(RpcErr::WrongParam("topics".to_string())),
                        }
                    })?;
                Ok(LogsFilter {
                    from_block,
                    to_block,
                    address_filters,
                    topics: topics_filters.unwrap_or_else(Vec::new),
                })
            }
            _ => Err(RpcErr::BadParams(
                "Params are not an array of one element".to_owned(),
            )),
        }
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let filtered_logs = fetch_logs_with_filter(self, context.storage).await?;
        serde_json::to_value(filtered_logs).map_err(|error| {
            tracing::error!("Log filtering request failed with: {error}");
            RpcErr::Internal("Failed to filter logs".to_string())
        })
    }
}

// TODO: This is longer than it has the right to be, maybe we should refactor it.
// The main problem here is the layers of indirection needed
// to fetch tx and block data for a log rpc response, some ideas here are:
// - The ideal one is to have a key-value store BlockNumber -> Log, where the log also stores
//   the block hash, transaction hash, transaction number and its own index.
// - Another on is the receipt stores the block hash, transaction hash and block number,
//   then we simply could retrieve each log from the receipt and add the info
//   needed for the RPCLog struct.

pub(crate) async fn fetch_logs_with_filter(
    filter: &LogsFilter,
    storage: Store,
) -> Result<Vec<RpcLog>, RpcErr> {
    let from = filter
        .from_block
        .resolve_block_number(&storage)
        .await?
        .ok_or(RpcErr::WrongParam("fromBlock".to_string()))?;
    let to = filter
        .to_block
        .resolve_block_number(&storage)
        .await?
        .ok_or(RpcErr::WrongParam("toBlock".to_string()))?;
    // Malformed user range (e.g. fromBlock=100, toBlock=50) — reject before
    // clamping so we don't silently accept inverted ranges.
    if from > to {
        return Err(RpcErr::BadParams("Empty range".to_string()));
    }
    // Clamp `from` up to the earliest unpruned block so that we don't attempt
    // to load block bodies / receipts that have already been pruned.
    let earliest = storage.get_earliest_block_number().await?;
    let from = from.max(earliest);
    // Entire requested range is below the prune cutoff — no logs to return.
    if from > to {
        return Ok(vec![]);
    }
    let address_filter: HashSet<_> = match &filter.address_filters {
        Some(AddressFilter::Single(address)) => std::iter::once(address).collect(),
        Some(AddressFilter::Many(addresses)) => addresses.iter().collect(),
        None => HashSet::new(),
    };

    let mut logs: Vec<RpcLog> = Vec::new();
    // The idea here is to fetch every log and filter by address, if given.
    // For that, we'll need each block in range, and its transactions,
    // and for each transaction, we'll need its receipts, which
    // contain the actual logs we want.
    for block_num in from..=to {
        // The block header carries a bloom filter over every (address, topic)
        // pair logged in the block. If it can't possibly contain a log matching
        // this filter, skip the block without loading its body or receipts.
        let block_header = storage
            .get_block_header(block_num)?
            .ok_or(RpcErr::Internal(format!(
                "Could not get header for block {block_num}"
            )))?;
        if !block_bloom_matches(&block_header.logs_bloom, &address_filter, &filter.topics) {
            continue;
        }
        // Take the body of the block, we
        // will use it to access the transactions.
        let block_body = storage
            .get_block_body(block_num)
            .await?
            .ok_or(RpcErr::Internal(format!(
                "Could not get body for block {block_num}"
            )))?;
        let block_hash = block_header.hash();

        // Fetch all of the block's receipts in a single bulk read instead of a
        // point lookup per transaction (each of which also re-resolved the
        // canonical block hash). For mainnet blocks with hundreds of txs this
        // is the dominant cost of eth_getLogs.
        let receipts = storage.get_receipts_for_block(&block_hash).await?;

        let mut block_log_index = 0_u64;

        // Transactions share indices with their receipts; pair them by index.
        for (tx_index, tx) in block_body.transactions.iter().enumerate() {
            let tx_hash = tx.hash(&NativeCrypto);
            let receipt = receipts.get(tx_index).ok_or(RpcErr::Internal(format!(
                "Missing receipt for block {block_num} tx {tx_index}"
            )))?;

            if receipt.succeeded {
                for log in &receipt.logs {
                    if address_filter.is_empty() || address_filter.contains(&log.address) {
                        // Some extra data is needed when
                        // forming the RPC response.
                        logs.push(RpcLog {
                            log: log.clone().into(),
                            log_index: block_log_index,
                            transaction_hash: tx_hash,
                            transaction_index: tx_index as u64,
                            block_number: block_num,
                            block_hash,
                            removed: false,
                        });
                    }
                    block_log_index += 1;
                }
            }
        }
    }
    // Now that we have the logs filtered by address,
    // we still need to filter by topics if it was a given parameter.

    let filtered_logs = if filter.topics.is_empty() {
        logs
    } else {
        logs.into_iter()
            .filter(|rpc_log| {
                if filter.topics.len() > rpc_log.log.topics.len() {
                    return false;
                }
                for (i, topic_filter) in filter.topics.iter().enumerate() {
                    match topic_filter {
                        TopicFilter::Topic(topic) => {
                            if topic.is_some_and(|topic| rpc_log.log.topics[i] != topic) {
                                return false;
                            }
                        }
                        TopicFilter::Topics(sub_topics) => {
                            if !sub_topics.is_empty()
                                && !sub_topics
                                    .iter()
                                    .any(|st| st.is_none_or(|t| rpc_log.log.topics[i] == t))
                            {
                                return false;
                            }
                        }
                    }
                }
                true
            })
            .collect::<Vec<RpcLog>>()
    };

    Ok(filtered_logs)
}

/// Necessary-condition check: returns `true` if the block's header bloom could
/// contain a log matching the filter, `false` only when it provably cannot.
///
/// A log matches when its address is one of the requested addresses (or none
/// were requested) AND, for every constrained topic position, the log's topic
/// equals one of the allowed values. Since the header bloom records every
/// logged address and topic (position-agnostic), a matching log implies its
/// address and each constrained topic are present in the bloom. We therefore
/// require: at least one requested address present (if any), and at least one
/// allowed topic present for each constrained position. Bloom false positives
/// are fine — exact filtering still runs on the blocks we don't skip.
fn block_bloom_matches(
    bloom: &Bloom,
    address_filter: &HashSet<&H160>,
    topics: &[TopicFilter],
) -> bool {
    if !address_filter.is_empty()
        && !address_filter
            .iter()
            .any(|address| bloom.contains_input(BloomInput::Raw(address.as_bytes())))
    {
        return false;
    }

    let topic_in_bloom = |topic: &H256| bloom.contains_input(BloomInput::Raw(topic.as_bytes()));
    topics.iter().all(|topic_filter| match topic_filter {
        // A wildcard position imposes no constraint.
        TopicFilter::Topic(None) => true,
        TopicFilter::Topic(Some(topic)) => topic_in_bloom(topic),
        // An empty alternatives list, or one containing any `None`, is a
        // wildcard for this position (the `None` means "any topic" — without
        // it, `topics: [[null, T]]` would skip blocks matching via the wildcard
        // and drop valid logs). Otherwise OR over the concrete alternatives.
        TopicFilter::Topics(sub_topics) => {
            sub_topics.is_empty()
                || sub_topics.iter().any(Option::is_none)
                || sub_topics.iter().flatten().any(topic_in_bloom)
        }
    })
}

#[cfg(test)]
mod pruning_log_tests {
    use super::*;
    use crate::test_utils::setup_store;
    use ethrex_common::types::{Block, BlockBody, BlockHeader};

    /// Build a canonical chain of empty blocks 1..=`count` on top of the genesis
    /// block that `setup_store` already inserted, using `forkchoice_update`.
    async fn add_empty_canonical_blocks(storage: &Store, count: u64) {
        let mut new_canonical = vec![];
        let mut parent = {
            let h = storage.get_block_header(0).unwrap().unwrap();
            h.hash()
        };
        for n in 1..=count {
            let header = BlockHeader {
                number: n,
                parent_hash: parent,
                ..Default::default()
            };
            let hash = header.hash();
            let block = Block::new(
                header,
                BlockBody {
                    transactions: vec![],
                    ommers: vec![],
                    withdrawals: Some(vec![]),
                },
            );
            storage.add_block(block).await.unwrap();
            new_canonical.push((n, hash));
            parent = hash;
        }
        let (last_num, last_hash) = new_canonical.pop().unwrap();
        storage
            .forkchoice_update(new_canonical, last_num, last_hash, None, None)
            .await
            .unwrap();
    }

    /// When `fromBlock` is below `earliest_block_number`, the handler must clamp
    /// `from` up to `earliest` and return logs from the unpruned range rather than
    /// erroring on the missing pruned bodies.
    #[tokio::test]
    async fn get_logs_clamps_from_block_to_earliest() {
        // setup_store() initialises genesis (block 0) and sets earliest = 0.
        let storage = setup_store().await;
        add_empty_canonical_blocks(&storage, 5).await;

        // Prune blocks 0–2 and move the earliest pointer to 3.
        for n in 0..=2u64 {
            storage.prune_block_height(n).await.unwrap();
        }
        storage.update_earliest_block_number(3).await.unwrap();
        assert_eq!(storage.get_earliest_block_number().await.unwrap(), 3);

        // Ask for logs from 0 to 5 — fromBlock is below earliest, so it must be
        // clamped to 3.  Blocks 3–5 have no transactions, so the result is empty
        // but the call must not error.
        let filter = LogsFilter {
            from_block: BlockIdentifier::Number(0),
            to_block: BlockIdentifier::Number(5),
            address_filters: None,
            topics: vec![],
        };
        let result = fetch_logs_with_filter(&filter, storage).await;
        assert!(result.is_ok(), "expected Ok, got {:?}", result.unwrap_err());
        assert!(
            result.unwrap().is_empty(),
            "expected no logs for empty blocks"
        );
    }

    /// When the entire requested range is below `earliest_block_number` (from > to
    /// after clamping), the handler must return an empty list, not an error.
    #[tokio::test]
    async fn get_logs_entire_range_below_earliest_returns_empty() {
        let storage = setup_store().await;

        // Set earliest to 10; ask for 0..=5 — entirely pruned.
        storage.update_earliest_block_number(10).await.unwrap();

        let filter = LogsFilter {
            from_block: BlockIdentifier::Number(0),
            to_block: BlockIdentifier::Number(5),
            address_filters: None,
            topics: vec![],
        };
        let result = fetch_logs_with_filter(&filter, storage).await;
        assert!(result.is_ok(), "expected Ok, got {:?}", result.unwrap_err());
        assert!(result.unwrap().is_empty());
    }

    /// A malformed inverted range (fromBlock > toBlock) must be rejected with
    /// `BadParams` even after the clamp logic was introduced — the clamp only
    /// suppresses errors for ranges that fall below the prune cutoff, not for
    /// user-supplied nonsense.
    #[tokio::test]
    async fn get_logs_rejects_malformed_inverted_range() {
        let storage = setup_store().await;

        let filter = LogsFilter {
            from_block: BlockIdentifier::Number(100),
            to_block: BlockIdentifier::Number(50),
            address_filters: None,
            topics: vec![],
        };
        let result = fetch_logs_with_filter(&filter, storage).await;
        assert!(
            matches!(result, Err(RpcErr::BadParams(_))),
            "expected BadParams, got {:?}",
            result
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_logs_with_defaults() {
        let params = Some(vec![
            json!({"topics": ["0x0000000000000000000000000000000000000000000000000000000000000000"]}),
        ]);
        let request = LogsFilter::parse(&params).unwrap();

        assert!(request.address_filters.is_none(), "{request:?}");
        assert!(
            matches!(request.from_block, BlockIdentifier::Tag(BlockTag::Latest)),
            "{request:?}"
        );
        assert!(
            matches!(request.to_block, BlockIdentifier::Tag(BlockTag::Latest)),
            "{request:?}"
        );
        assert_eq!(request.topics, vec![TopicFilter::Topic(Some(H256::zero()))]);
    }

    #[test]
    fn test_get_logs_multiple_addresses() {
        let params = Some(vec![json!({
            "address": [
                "0x0000000000000000000000000000000000000001",
                "0x0000000000000000000000000000000000000002"
            ],
            "topics": ["0x0000000000000000000000000000000000000000000000000000000000000000"]
        })]);
        let request = LogsFilter::parse(&params).unwrap();

        assert_eq!(
            request.address_filters.as_ref().unwrap().as_ref(),
            [H160::from_low_u64_be(1), H160::from_low_u64_be(2)],
        );
        assert!(
            matches!(request.from_block, BlockIdentifier::Tag(BlockTag::Latest)),
            "{request:?}"
        );
        assert!(
            matches!(request.to_block, BlockIdentifier::Tag(BlockTag::Latest)),
            "{request:?}"
        );
        assert_eq!(request.topics, vec![TopicFilter::Topic(Some(H256::zero()))]);
    }

    fn addr(n: u64) -> H160 {
        H160::from_low_u64_be(n)
    }

    fn topic(n: u64) -> H256 {
        H256::from_low_u64_be(n)
    }

    /// Builds a header bloom the same way the block producer does: by accruing
    /// every address and topic of every log (see `bloom_from_logs`).
    fn bloom_with(addresses: &[H160], topics: &[H256]) -> Bloom {
        let mut bloom = Bloom::zero();
        for address in addresses {
            bloom.accrue(BloomInput::Raw(address.as_bytes()));
        }
        for topic in topics {
            bloom.accrue(BloomInput::Raw(topic.as_bytes()));
        }
        bloom
    }

    fn addr_set(addresses: &[H160]) -> HashSet<&H160> {
        addresses.iter().collect()
    }

    #[test]
    fn bloom_match_empty_filter_always_matches() {
        // No address and no topic constraints: never skip a block.
        assert!(block_bloom_matches(&Bloom::zero(), &HashSet::new(), &[]));
    }

    #[test]
    fn bloom_match_address_present_and_absent() {
        let bloom = bloom_with(&[addr(1)], &[]);
        assert!(block_bloom_matches(&bloom, &addr_set(&[addr(1)]), &[]));
        assert!(!block_bloom_matches(&bloom, &addr_set(&[addr(2)]), &[]));
    }

    #[test]
    fn bloom_match_multiple_addresses_is_or() {
        let bloom = bloom_with(&[addr(1)], &[]);
        // Only one of the requested addresses needs to be present.
        assert!(block_bloom_matches(
            &bloom,
            &addr_set(&[addr(1), addr(2)]),
            &[]
        ));
        assert!(!block_bloom_matches(
            &bloom,
            &addr_set(&[addr(2), addr(3)]),
            &[]
        ));
    }

    #[test]
    fn bloom_match_topic_present_and_absent() {
        let bloom = bloom_with(&[], &[topic(1)]);
        assert!(block_bloom_matches(
            &bloom,
            &HashSet::new(),
            &[TopicFilter::Topic(Some(topic(1)))]
        ));
        assert!(!block_bloom_matches(
            &bloom,
            &HashSet::new(),
            &[TopicFilter::Topic(Some(topic(2)))]
        ));
    }

    #[test]
    fn bloom_match_wildcard_topic_ignored() {
        // A `None` (wildcard) topic position imposes no constraint.
        assert!(block_bloom_matches(
            &Bloom::zero(),
            &HashSet::new(),
            &[TopicFilter::Topic(None)]
        ));
        assert!(block_bloom_matches(
            &Bloom::zero(),
            &HashSet::new(),
            &[TopicFilter::Topics(vec![])]
        ));
    }

    #[test]
    fn bloom_match_topics_with_none_element_is_wildcard() {
        // A `None` inside a `Topics([...])` alternatives list means "any topic"
        // at this position, so the position is a wildcard and must not be skipped
        // even when the sibling topic is absent from the bloom. Regression test for
        // a false-negative that dropped valid logs for `topics: [[null, T]]` queries.
        let bloom = bloom_with(&[], &[]); // contains neither topic
        assert!(block_bloom_matches(
            &bloom,
            &HashSet::new(),
            &[TopicFilter::Topics(vec![Some(topic(2)), None])]
        ));
    }

    #[test]
    fn bloom_match_topic_position_is_or_across_positions_is_and() {
        let bloom = bloom_with(&[], &[topic(1), topic(2)]);
        // OR within a position: any allowed value present is enough.
        assert!(block_bloom_matches(
            &bloom,
            &HashSet::new(),
            &[TopicFilter::Topics(vec![Some(topic(2)), Some(topic(9))])]
        ));
        // AND across positions: every constrained position must be satisfied.
        assert!(block_bloom_matches(
            &bloom,
            &HashSet::new(),
            &[
                TopicFilter::Topic(Some(topic(1))),
                TopicFilter::Topic(Some(topic(2))),
            ]
        ));
        assert!(!block_bloom_matches(
            &bloom,
            &HashSet::new(),
            &[
                TopicFilter::Topic(Some(topic(1))),
                TopicFilter::Topic(Some(topic(9))),
            ]
        ));
    }

    #[test]
    fn bloom_match_requires_both_address_and_topic() {
        let bloom = bloom_with(&[addr(1)], &[topic(1)]);
        assert!(block_bloom_matches(
            &bloom,
            &addr_set(&[addr(1)]),
            &[TopicFilter::Topic(Some(topic(1)))]
        ));
        // Address matches but topic does not.
        assert!(!block_bloom_matches(
            &bloom,
            &addr_set(&[addr(1)]),
            &[TopicFilter::Topic(Some(topic(2)))]
        ));
    }
}
