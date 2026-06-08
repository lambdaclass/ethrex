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
    if (from..=to).is_empty() {
        return Err(RpcErr::BadParams("Empty range".to_string()));
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

        let mut block_log_index = 0_u64;

        // Since transactions share indices with their receipts,
        // we'll use them to fetch their receipts, which have the actual logs.
        for (tx_index, tx) in block_body.transactions.iter().enumerate() {
            let tx_hash = tx.hash();
            let receipt = storage
                .get_receipt(block_num, tx_index as u64)
                .await?
                .ok_or(RpcErr::Internal("Could not get receipt".to_owned()))?;

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

    topics.iter().all(|topic_filter| {
        let allowed: Vec<&H256> = match topic_filter {
            TopicFilter::Topic(topic) => topic.iter().collect(),
            TopicFilter::Topics(sub_topics) => sub_topics.iter().flatten().collect(),
        };
        // An empty set of allowed topics is a wildcard for this position.
        allowed.is_empty()
            || allowed
                .iter()
                .any(|topic| bloom.contains_input(BloomInput::Raw(topic.as_bytes())))
    })
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
