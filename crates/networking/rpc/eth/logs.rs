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
use ethrex_common::{
    H160, H256,
    types::{BlockBody, BlockHeader, BlockNumber},
};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::Store;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

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
    /// Restricts the filter to a single block, by hash. The spec defines this as
    /// an alternative to the `fromBlock`/`toBlock` range, so the two forms are
    /// mutually exclusive and this one takes precedence when present.
    pub block_hash: Option<H256>,
    /// The addresses from where the logs origin from.
    pub address_filters: Option<AddressFilter>,
    /// Which topics to filter. Empty means "match any topic", which is also what
    /// an absent `topics` field means.
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
                let block_hash = param
                    .get("blockHash")
                    .map(
                        |block_hash| match serde_json::from_value::<H256>(block_hash.clone()) {
                            Ok(hash) => Ok(hash),
                            _ => Err(RpcErr::WrongParam("blockHash".to_string())),
                        },
                    )
                    .transpose()?;
                // Every field of the filter object is optional in the spec, so an
                // absent `topics` means "match any topic" rather than an error.
                let topics_filters = param
                    .get("topics")
                    .map(|topics| {
                        match serde_json::from_value::<Option<Vec<TopicFilter>>>(topics.clone()) {
                            Ok(filters) => Ok(filters),
                            _ => Err(RpcErr::WrongParam("topics".to_string())),
                        }
                    })
                    .transpose()?
                    .flatten();
                // The spec models the filter as one of two shapes: a block range,
                // or a single block by hash. Combining them is a malformed request
                // rather than a silent precedence rule.
                if block_hash.is_some()
                    && (param.contains_key("fromBlock") || param.contains_key("toBlock"))
                {
                    return Err(RpcErr::BadParams(
                        "`blockHash` cannot be combined with `fromBlock` or `toBlock`".to_string(),
                    ));
                }
                Ok(LogsFilter {
                    from_block,
                    to_block,
                    block_hash,
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
        let filtered_logs =
            fetch_logs_with_filter(self, context.storage, &context.log_query_limits).await?;
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

/// Blocks whose headers one blocking task reads and checks against the filter's bloom,
/// the same chunk size reth uses for its header pass.
const HEADER_CHUNK_SIZE: u64 = 1_000;

/// Default for `--rpc.max-blocks-per-filter`, the same as reth.
pub const DEFAULT_MAX_BLOCKS_PER_FILTER: u64 = 100_000;

/// Default for `--rpc.max-logs-per-response`, the same as reth.
pub const DEFAULT_MAX_LOGS_PER_RESPONSE: u64 = 20_000;

/// Default for `--rpc.max-log-query-work`: a query over the full default range may check
/// 100 address and topic combinations, and a 10,000-block one may check 1,000 addresses,
/// geth's per-filter address limit.
pub const DEFAULT_MAX_LOG_QUERY_WORK: u64 = 100 * DEFAULT_MAX_BLOCKS_PER_FILTER;

/// Default for `--rpc.log-query-work-budget`: room for four queries at the work limit.
pub const DEFAULT_LOG_QUERY_WORK_BUDGET: u64 = 4 * DEFAULT_MAX_LOG_QUERY_WORK;

/// Queries with at most this much work, such as 10,000 blocks for one address and topic,
/// skip the work budget. They are cheap, and keeping them out means ordinary queries
/// never get a budget error while heavy ones hold it.
const LIGHT_LOG_QUERY_WORK: u64 = 10_000;

/// What one log query may cost. `eth_getLogs` and `eth_getFilterChanges` share it, and
/// clones share the work budget.
#[derive(Clone, Debug)]
pub struct LogQueryLimits {
    /// Largest `to - from` of a range query; `None` for no limit.
    max_blocks_per_filter: Option<u64>,
    /// Most logs a query spanning several blocks may return; `None` for no limit.
    max_logs_per_response: Option<usize>,
    /// Largest scan-work estimate (see [`LogQueryWork`]) a query may have; `None` for no
    /// limit.
    max_log_query_work: Option<u64>,
    /// Work of the heavy queries in flight. Each takes its estimate, capped at the whole
    /// budget, and gives it back when it finishes, fails, or is dropped.
    work_budget: Arc<Semaphore>,
    /// Permits in `work_budget` while no query holds any.
    work_budget_size: usize,
}

impl LogQueryLimits {
    /// Zero for any of the first three limits means no limit. A zero `work_budget`
    /// refuses every query above the light threshold.
    pub fn new(
        max_blocks_per_filter: u64,
        max_logs_per_response: u64,
        max_log_query_work: u64,
        work_budget: u64,
    ) -> Self {
        let work_budget_size = usize::try_from(work_budget)
            .unwrap_or(usize::MAX)
            .min(Semaphore::MAX_PERMITS);
        Self {
            max_blocks_per_filter: (max_blocks_per_filter != 0).then_some(max_blocks_per_filter),
            max_logs_per_response: (max_logs_per_response != 0)
                .then(|| usize::try_from(max_logs_per_response).unwrap_or(usize::MAX)),
            max_log_query_work: (max_log_query_work != 0).then_some(max_log_query_work),
            work_budget: Arc::new(Semaphore::new(work_budget_size)),
            work_budget_size,
        }
    }

    /// Rejects a range query spanning more blocks than allowed, with reth's message.
    fn check_block_range(&self, from: BlockNumber, to: BlockNumber) -> Result<(), RpcErr> {
        match self.max_blocks_per_filter {
            Some(max) if to - from > max => Err(RpcErr::InvalidParams(format!(
                "query exceeds max block range {max}"
            ))),
            _ => Ok(()),
        }
    }

    /// Rejects a query whose work estimate is over the limit.
    fn check_work(&self, work: &LogQueryWork) -> Result<(), RpcErr> {
        match self.max_log_query_work {
            Some(max) if work.total() > max => Err(RpcErr::LimitExceeded(format!(
                "log query too expensive (blocks {} x addresses {} x topic alternatives {} = {} units of work, limit {max}); narrow the block range or the filter",
                work.blocks,
                work.addresses,
                work.topics,
                work.total()
            ))),
            _ => Ok(()),
        }
    }

    /// Takes the query's work from the budget for as long as the returned permit lives.
    /// Light queries take nothing. Fails at once when the budget has no room, instead of
    /// queueing.
    fn reserve(&self, work: &LogQueryWork) -> Result<Option<OwnedSemaphorePermit>, RpcErr> {
        let work = work.total();
        if work <= LIGHT_LOG_QUERY_WORK {
            return Ok(None);
        }
        // A query never needs more than the whole budget, and at least one permit, so an
        // empty budget refuses it.
        let permits = usize::try_from(work)
            .unwrap_or(usize::MAX)
            .min(self.work_budget_size)
            .max(1);
        let permits = u32::try_from(permits).unwrap_or(u32::MAX);
        self.work_budget
            .clone()
            .try_acquire_many_owned(permits)
            .map(Some)
            .map_err(|_| {
                RpcErr::LimitExceeded("too many log queries in flight, retry later".to_string())
            })
    }

    /// Rejects a filter that could not scan even one block within the work limit, so
    /// `eth_newFilter` refuses it up front instead of failing every poll.
    pub(crate) fn check_filter(&self, filter: &LogsFilter) -> Result<(), RpcErr> {
        self.check_work(&LogQueryWork::new(1, &filter.address_set(), &filter.topics))
    }
}

impl Default for LogQueryLimits {
    fn default() -> Self {
        Self::new(
            DEFAULT_MAX_BLOCKS_PER_FILTER,
            DEFAULT_MAX_LOGS_PER_RESPONSE,
            DEFAULT_MAX_LOG_QUERY_WORK,
            DEFAULT_LOG_QUERY_WORK_BUDGET,
        )
    }
}

/// Upper bound on the scan work of a log query: the blocks it scans, times the addresses
/// and the topic alternatives each block is checked against. Each extra address or
/// alternative can make more blocks match, and every matching block costs a body and a
/// receipts read. Only the largest OR-set counts: topic positions are ANDed, so another
/// constrained position can only narrow the matches.
struct LogQueryWork {
    blocks: u64,
    /// Distinct addresses, at least 1.
    addresses: u64,
    /// Alternatives in the largest constrained OR-set, at least 1.
    topics: u64,
}

impl LogQueryWork {
    fn new(blocks: u64, addresses: &HashSet<&H160>, topics: &[TopicFilter]) -> Self {
        let largest_or_set = topics
            .iter()
            .map(|position| match position {
                // A list containing `null` is a wildcard, like a single `null`.
                TopicFilter::Topics(alternatives) if !alternatives.contains(&None) => {
                    alternatives.len()
                }
                TopicFilter::Topics(_) | TopicFilter::Topic(_) => 1,
            })
            .max()
            .unwrap_or(1);
        Self {
            blocks,
            addresses: u64::try_from(addresses.len().max(1)).unwrap_or(u64::MAX),
            topics: u64::try_from(largest_or_set.max(1)).unwrap_or(u64::MAX),
        }
    }

    fn total(&self) -> u64 {
        self.blocks
            .saturating_mul(self.addresses)
            .saturating_mul(self.topics)
    }
}

impl LogsFilter {
    /// The filter's addresses, without repeats; empty when it has none.
    fn address_set(&self) -> HashSet<&H160> {
        match &self.address_filters {
            Some(AddressFilter::Single(address)) => std::iter::once(address).collect(),
            Some(AddressFilter::Many(addresses)) => addresses.iter().collect(),
            None => HashSet::new(),
        }
    }
}

pub(crate) async fn fetch_logs_with_filter(
    filter: &LogsFilter,
    storage: Store,
    limits: &LogQueryLimits,
) -> Result<Vec<RpcLog>, RpcErr> {
    let address_filter = filter.address_set();
    // Derive the filter's address/topic blooms once, up front, so the per-block
    // header-bloom check below is a cheap bit-subset test instead of re-hashing
    // every address and topic for each block in the range.
    let bloom_matcher = Arc::new(BloomFilterMatcher::new(&address_filter, &filter.topics));

    let mut logs: Vec<RpcLog> = Vec::new();
    match filter.block_hash {
        Some(block_hash) => {
            let block_header = storage
                .get_block_header_by_hash(block_hash)?
                .ok_or_else(|| RpcErr::BadParams(format!("Unknown block hash {block_hash:#x}")))?;
            let work = LogQueryWork::new(1, &address_filter, &filter.topics);
            limits.check_work(&work)?;
            let _reservation = limits.reserve(&work)?;
            if bloom_matcher.matches(&block_header.logs_bloom) {
                let block_body =
                    storage
                        .get_block_body_by_hash(block_hash)
                        .await?
                        .ok_or(RpcErr::Internal(format!(
                            "Could not get body for block {block_hash:#x}"
                        )))?;
                collect_block_logs(
                    &storage,
                    &block_header,
                    &block_body,
                    &address_filter,
                    &filter.topics,
                    &mut logs,
                )
                .await?;
            }
        }
        None => {
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
            let latest = storage.get_latest_block_number()?;
            if from > to {
                return Err(RpcErr::InvalidParams(
                    "invalid block range params".to_string(),
                ));
            }
            if to > latest {
                return Err(RpcErr::InvalidParams(
                    "block range extends beyond current head block".to_string(),
                ));
            }
            limits.check_block_range(from, to)?;
            let work = LogQueryWork::new(to - from + 1, &address_filter, &filter.topics);
            limits.check_work(&work)?;
            let _reservation = limits.reserve(&work)?;
            // A single block cannot be split any further, so it is exempt from the cap on
            // results, as in reth.
            let max_logs = limits.max_logs_per_response.filter(|_| from != to);
            // The idea here is to fetch every log and filter by address, if given.
            // For that, we'll need each block in range, and its transactions,
            // and for each transaction, we'll need its receipts, which
            // contain the actual logs we want.
            let mut chunk_start = from;
            while chunk_start <= to {
                let chunk_end = chunk_start.saturating_add(HEADER_CHUNK_SIZE - 1).min(to);
                let headers = bloom_matching_headers(
                    storage.clone(),
                    bloom_matcher.clone(),
                    chunk_start,
                    chunk_end,
                )
                .await?;
                for block_header in headers {
                    let block_num = block_header.number;
                    // Take the body of the block, we
                    // will use it to access the transactions.
                    let block_body =
                        storage
                            .get_block_body(block_num)
                            .await?
                            .ok_or(RpcErr::Internal(format!(
                                "Could not get body for block {block_num}"
                            )))?;
                    collect_block_logs(
                        &storage,
                        &block_header,
                        &block_body,
                        &address_filter,
                        &filter.topics,
                        &mut logs,
                    )
                    .await?;
                    if let Some(max) = max_logs
                        && logs.len() > max
                    {
                        // Suggest the blocks scanned in full before this one, as reth does,
                        // so clients can split the range from there.
                        let last_complete = block_num.saturating_sub(1).max(from);
                        return Err(RpcErr::InvalidParams(format!(
                            "query exceeds max results {max}, retry with the range {from}-{last_complete}"
                        )));
                    }
                }
                let Some(next) = chunk_end.checked_add(1) else {
                    break;
                };
                chunk_start = next;
            }
        }
    }
    Ok(logs)
}

/// Reads the canonical headers of blocks `from..=to` on a blocking thread and returns
/// those whose bloom could hold a log matching the filter, in block order. The header
/// bloom covers every address and topic logged in the block, so a block it rules out is
/// skipped without loading its body or receipts. These reads are synchronous, and for
/// blocks the bloom rules out they are all the work there is, so they stay off the async
/// executor.
async fn bloom_matching_headers(
    storage: Store,
    bloom_matcher: Arc<BloomFilterMatcher>,
    from: BlockNumber,
    to: BlockNumber,
) -> Result<Vec<BlockHeader>, RpcErr> {
    tokio::task::spawn_blocking(move || {
        let mut matching = Vec::new();
        for block_num in from..=to {
            let block_header = storage
                .get_block_header(block_num)?
                .ok_or(RpcErr::Internal(format!(
                    "Could not get header for block {block_num}"
                )))?;
            if bloom_matcher.matches(&block_header.logs_bloom) {
                matching.push(block_header);
            }
        }
        Ok(matching)
    })
    .await
    .map_err(|error| RpcErr::Internal(format!("Log scan task failed: {error}")))?
}

/// Collect every log of one block that matches the filter's addresses and topics into
/// `logs`, pairing the block's transactions with their receipts by index.
async fn collect_block_logs(
    storage: &Store,
    block_header: &BlockHeader,
    block_body: &BlockBody,
    address_filter: &HashSet<&H160>,
    topic_filter: &[TopicFilter],
    logs: &mut Vec<RpcLog>,
) -> Result<(), RpcErr> {
    let block_num = block_header.number;
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
                if (address_filter.is_empty() || address_filter.contains(&log.address))
                    && matches_topics(&log.topics, topic_filter)
                {
                    // Some extra data is needed when
                    // forming the RPC response.
                    logs.push(RpcLog {
                        log: log.clone().into(),
                        log_index: block_log_index,
                        transaction_hash: tx_hash,
                        transaction_index: tx_index as u64,
                        block_number: block_num,
                        block_hash,
                        block_timestamp: block_header.timestamp,
                        removed: false,
                    });
                }
                block_log_index += 1;
            }
        }
    }
    Ok(())
}

/// Whether a log with `topics` matches `topic_filter`: each filter position is a
/// wildcard (`null`, an empty list, or a list containing `null`) or a set of allowed
/// values for the log's topic at that position, and the log must have a topic at every
/// filter position.
fn matches_topics(topics: &[H256], topic_filter: &[TopicFilter]) -> bool {
    if topic_filter.len() > topics.len() {
        return false;
    }
    topic_filter
        .iter()
        .zip(topics)
        .all(|(position, topic)| match position {
            TopicFilter::Topic(expected) => expected.is_none_or(|expected| *topic == expected),
            TopicFilter::Topics(alternatives) => {
                alternatives.is_empty()
                    || alternatives
                        .iter()
                        .any(|alternative| alternative.is_none_or(|t| *topic == t))
            }
        })
}

/// A log filter's addresses and topic positions pre-derived into header-bloom
/// `Bloom`s once, so the per-block check is a cheap bit-subset test rather than
/// re-hashing every address/topic for each block in the range.
///
/// `Bloom::contains_input` internally builds a `Bloom` from the input (a keccak
/// hash plus bit extraction) before testing it; for wide-range queries that skip
/// most blocks that derivation dominates the per-block cost. We do it once here.
struct BloomFilterMatcher {
    /// One bloom per requested address; empty means no address constraint.
    addresses: Vec<Bloom>,
    /// One entry per *constrained* topic position, each holding that position's
    /// alternatives; at least one must be present. Wildcard positions impose no
    /// constraint and are dropped (a no-op in the all-positions check), so the
    /// position index is irrelevant — the header bloom is position-agnostic.
    topic_positions: Vec<Vec<Bloom>>,
}

impl BloomFilterMatcher {
    fn new(address_filter: &HashSet<&H160>, topics: &[TopicFilter]) -> Self {
        let to_bloom = |bytes: &[u8]| Bloom::from(BloomInput::Raw(bytes));
        let addresses = address_filter
            .iter()
            .map(|address| to_bloom(address.as_bytes()))
            .collect();
        let topic_positions = topics
            .iter()
            .filter_map(|topic_filter| match topic_filter {
                // A wildcard position imposes no constraint; drop it.
                TopicFilter::Topic(None) => None,
                TopicFilter::Topic(Some(topic)) => Some(vec![to_bloom(topic.as_bytes())]),
                // An empty alternatives list, or one containing any `None`, is a
                // wildcard for this position (the `None` means "any topic" —
                // without it, `topics: [[null, T]]` would skip blocks matching
                // via the wildcard and drop valid logs); drop it.
                TopicFilter::Topics(sub_topics)
                    if sub_topics.is_empty() || sub_topics.iter().any(Option::is_none) =>
                {
                    None
                }
                // Otherwise OR over the concrete alternatives.
                TopicFilter::Topics(sub_topics) => Some(
                    sub_topics
                        .iter()
                        .flatten()
                        .map(|topic| to_bloom(topic.as_bytes()))
                        .collect(),
                ),
            })
            .collect();
        Self {
            addresses,
            topic_positions,
        }
    }

    /// Necessary-condition check: returns `true` if the block's header bloom
    /// could contain a log matching the filter, `false` only when it provably
    /// cannot.
    ///
    /// A log matches when its address is one of the requested addresses (or none
    /// were requested) AND, for every constrained topic position, the log's
    /// topic equals one of the allowed values. Since the header bloom records
    /// every logged address and topic (position-agnostic), a matching log
    /// implies its address and each constrained topic are present in the bloom.
    /// We therefore require: at least one requested address present (if any), and
    /// at least one allowed topic present for each constrained position. Bloom
    /// false positives are fine — exact filtering still runs on the blocks we
    /// don't skip.
    fn matches(&self, block_bloom: &Bloom) -> bool {
        if !self.addresses.is_empty()
            && !self
                .addresses
                .iter()
                .any(|address| block_bloom.contains_bloom(address))
        {
            return false;
        }
        self.topic_positions.iter().all(|alternatives| {
            alternatives
                .iter()
                .any(|topic| block_bloom.contains_bloom(topic))
        })
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
    fn test_get_logs_without_topics() {
        // Every field of the filter object is optional; an absent `topics` asks
        // for every log in range.
        let params = Some(vec![json!({"fromBlock": "0x1", "toBlock": "0x2"})]);
        let request = LogsFilter::parse(&params).unwrap();

        assert!(request.topics.is_empty(), "{request:?}");
        assert!(request.block_hash.is_none(), "{request:?}");
    }

    #[test]
    fn test_get_logs_with_empty_filter() {
        let params = Some(vec![json!({})]);
        let request = LogsFilter::parse(&params).unwrap();

        assert!(request.topics.is_empty(), "{request:?}");
        assert!(request.address_filters.is_none(), "{request:?}");
        assert!(
            matches!(request.from_block, BlockIdentifier::Tag(BlockTag::Latest)),
            "{request:?}"
        );
    }

    #[test]
    fn test_get_logs_by_block_hash() {
        let params = Some(vec![json!({
            "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000001"
        })]);
        let request = LogsFilter::parse(&params).unwrap();

        assert_eq!(request.block_hash, Some(H256::from_low_u64_be(1)));
        assert!(request.topics.is_empty(), "{request:?}");
    }

    #[test]
    fn test_get_logs_block_hash_with_range_is_rejected() {
        // The spec models the two forms as mutually exclusive.
        for extra in ["fromBlock", "toBlock"] {
            let params = Some(vec![json!({
                "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000001",
                extra: "0x1"
            })]);
            assert!(
                matches!(LogsFilter::parse(&params), Err(RpcErr::BadParams(_))),
                "expected {extra} alongside blockHash to be rejected"
            );
        }
    }

    #[test]
    fn test_get_logs_malformed_block_hash_is_rejected() {
        let params = Some(vec![json!({"blockHash": "not a hash"})]);
        assert!(matches!(
            LogsFilter::parse(&params),
            Err(RpcErr::WrongParam(_))
        ));
    }

    /// A `blockHash` filter must return the logs of exactly that block, even
    /// when the block is no longer canonical. Resolving the hash through the
    /// canonical number index would instead return the logs of whatever block
    /// replaced it at the same height after a reorg, silently, which is the
    /// substitution EIP-234 exists to prevent.
    #[tokio::test]
    async fn test_get_logs_rejects_out_of_range_blocks() {
        use ethrex_common::types::{Block, BlockBody};
        use ethrex_storage::EngineType;

        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        let block = Block::new(
            BlockHeader {
                number: 1,
                ..Default::default()
            },
            BlockBody::default(),
        );
        let hash = block.hash();
        storage.add_block(block).await.unwrap();
        storage
            .forkchoice_update(vec![(1, hash)], 1, hash, None, None)
            .await
            .unwrap();

        let filter_for = |from_block, to_block| LogsFilter {
            from_block,
            to_block,
            block_hash: None,
            address_filters: None,
            topics: vec![],
        };

        let err = fetch_logs_with_filter(
            &filter_for(BlockIdentifier::Number(1), BlockIdentifier::Number(3)),
            storage.clone(),
            &LogQueryLimits::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, RpcErr::InvalidParams(ref m) if m == "block range extends beyond current head block"),
            "{err:?}"
        );

        let err = fetch_logs_with_filter(
            &filter_for(
                BlockIdentifier::Number(2),
                BlockIdentifier::Tag(BlockTag::Latest),
            ),
            storage.clone(),
            &LogQueryLimits::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, RpcErr::InvalidParams(ref m) if m == "invalid block range params"),
            "{err:?}"
        );

        let err = fetch_logs_with_filter(
            &filter_for(BlockIdentifier::Number(1), BlockIdentifier::Number(0)),
            storage,
            &LogQueryLimits::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, RpcErr::InvalidParams(ref m) if m == "invalid block range params"),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn test_get_logs_by_block_hash_survives_reorg() {
        use ethrex_common::types::{Block, BlockBody, LegacyTransaction, Log, Receipt, TxType};
        use ethrex_storage::EngineType;

        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");

        // Two competing blocks at height 1, distinguished by their extra_data
        // and by the address their single log comes from.
        let make_block = |tag: u8| {
            let header = BlockHeader {
                number: 1,
                extra_data: vec![tag].into(),
                ..Default::default()
            };
            let body = BlockBody {
                transactions: vec![ethrex_common::types::Transaction::LegacyTransaction(
                    LegacyTransaction::default(),
                )],
                ommers: Default::default(),
                withdrawals: Default::default(),
            };
            Block::new(header, body)
        };
        let reorged_out = make_block(1);
        let canonical = make_block(2);
        let reorged_hash = reorged_out.hash();
        let canonical_hash = canonical.hash();
        assert_ne!(reorged_hash, canonical_hash);

        let receipt_logging_from = |address: H160| {
            Receipt::new(
                TxType::Legacy,
                true,
                21000,
                vec![Log {
                    address,
                    topics: vec![],
                    data: Default::default(),
                }],
            )
        };
        storage.add_block(reorged_out).await.unwrap();
        storage.add_block(canonical).await.unwrap();
        storage
            .add_receipts(
                reorged_hash,
                vec![receipt_logging_from(H160::repeat_byte(1))],
            )
            .await
            .unwrap();
        storage
            .add_receipts(
                canonical_hash,
                vec![receipt_logging_from(H160::repeat_byte(2))],
            )
            .await
            .unwrap();
        // Make the second block the canonical one at height 1, demoting the first.
        storage
            .forkchoice_update(vec![(1, canonical_hash)], 1, canonical_hash, None, None)
            .await
            .unwrap();

        let filter_for = |block_hash| LogsFilter {
            from_block: BlockIdentifier::Tag(BlockTag::Latest),
            to_block: BlockIdentifier::Tag(BlockTag::Latest),
            block_hash: Some(block_hash),
            address_filters: None,
            topics: vec![],
        };

        // The demoted block's hash returns the demoted block's own logs.
        let logs = fetch_logs_with_filter(
            &filter_for(reorged_hash),
            storage.clone(),
            &LogQueryLimits::default(),
        )
        .await
        .unwrap();
        assert_eq!(logs.len(), 1, "{logs:?}");
        assert_eq!(logs[0].block_hash, reorged_hash);
        assert_eq!(logs[0].log.address, H160::repeat_byte(1));

        // The canonical block's hash returns its logs, not the demoted one's.
        let logs = fetch_logs_with_filter(
            &filter_for(canonical_hash),
            storage.clone(),
            &LogQueryLimits::default(),
        )
        .await
        .unwrap();
        assert_eq!(logs.len(), 1, "{logs:?}");
        assert_eq!(logs[0].block_hash, canonical_hash);
        assert_eq!(logs[0].log.address, H160::repeat_byte(2));

        // A hash the node has never seen is an error, not an empty result.
        let unknown = H256::repeat_byte(0xff);
        assert!(matches!(
            fetch_logs_with_filter(&filter_for(unknown), storage, &LogQueryLimits::default()).await,
            Err(RpcErr::BadParams(_))
        ));
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

    fn bloom_matches(bloom: &Bloom, addresses: &HashSet<&H160>, topics: &[TopicFilter]) -> bool {
        BloomFilterMatcher::new(addresses, topics).matches(bloom)
    }

    #[test]
    fn bloom_match_empty_filter_always_matches() {
        // No address and no topic constraints: never skip a block.
        assert!(bloom_matches(&Bloom::zero(), &HashSet::new(), &[]));
    }

    #[test]
    fn bloom_match_address_present_and_absent() {
        let bloom = bloom_with(&[addr(1)], &[]);
        assert!(bloom_matches(&bloom, &addr_set(&[addr(1)]), &[]));
        assert!(!bloom_matches(&bloom, &addr_set(&[addr(2)]), &[]));
    }

    #[test]
    fn bloom_match_multiple_addresses_is_or() {
        let bloom = bloom_with(&[addr(1)], &[]);
        // Only one of the requested addresses needs to be present.
        assert!(bloom_matches(&bloom, &addr_set(&[addr(1), addr(2)]), &[]));
        assert!(!bloom_matches(&bloom, &addr_set(&[addr(2), addr(3)]), &[]));
    }

    #[test]
    fn bloom_match_topic_present_and_absent() {
        let bloom = bloom_with(&[], &[topic(1)]);
        assert!(bloom_matches(
            &bloom,
            &HashSet::new(),
            &[TopicFilter::Topic(Some(topic(1)))]
        ));
        assert!(!bloom_matches(
            &bloom,
            &HashSet::new(),
            &[TopicFilter::Topic(Some(topic(2)))]
        ));
    }

    #[test]
    fn bloom_match_wildcard_topic_ignored() {
        // A `None` (wildcard) topic position imposes no constraint.
        assert!(bloom_matches(
            &Bloom::zero(),
            &HashSet::new(),
            &[TopicFilter::Topic(None)]
        ));
        assert!(bloom_matches(
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
        assert!(bloom_matches(
            &bloom,
            &HashSet::new(),
            &[TopicFilter::Topics(vec![Some(topic(2)), None])]
        ));
    }

    #[test]
    fn bloom_match_topic_position_is_or_across_positions_is_and() {
        let bloom = bloom_with(&[], &[topic(1), topic(2)]);
        // OR within a position: any allowed value present is enough.
        assert!(bloom_matches(
            &bloom,
            &HashSet::new(),
            &[TopicFilter::Topics(vec![Some(topic(2)), Some(topic(9))])]
        ));
        // AND across positions: every constrained position must be satisfied.
        assert!(bloom_matches(
            &bloom,
            &HashSet::new(),
            &[
                TopicFilter::Topic(Some(topic(1))),
                TopicFilter::Topic(Some(topic(2))),
            ]
        ));
        assert!(!bloom_matches(
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
        assert!(bloom_matches(
            &bloom,
            &addr_set(&[addr(1)]),
            &[TopicFilter::Topic(Some(topic(1)))]
        ));
        // Address matches but topic does not.
        assert!(!bloom_matches(
            &bloom,
            &addr_set(&[addr(1)]),
            &[TopicFilter::Topic(Some(topic(2)))]
        ));
    }

    /// A transaction in a test block: whether its receipt succeeded, and its logs as
    /// `(address, topics)` built from `addr` and `topic`.
    type TestTx = (bool, Vec<(u64, Vec<u64>)>);

    /// An in-memory chain whose block `n` holds `blocks[n]`, with header blooms built from
    /// the successful receipts' logs, all canonical.
    async fn store_with_blocks(blocks: &[Vec<TestTx>]) -> Store {
        use ethrex_common::types::{
            Block, BlockBody, LegacyTransaction, Log, Receipt, Transaction, TxType, bloom_from_logs,
        };
        use ethrex_storage::EngineType;

        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        let mut canonical = Vec::new();
        for (number, txs) in blocks.iter().enumerate() {
            let number = number as u64;
            let mut transactions = Vec::new();
            let mut receipts = Vec::new();
            let mut logged = Vec::new();
            for (nonce, (succeeded, logs)) in txs.iter().enumerate() {
                transactions.push(Transaction::LegacyTransaction(LegacyTransaction {
                    nonce: number * 100 + nonce as u64,
                    ..Default::default()
                }));
                let logs: Vec<Log> = logs
                    .iter()
                    .map(|(address, topics)| Log {
                        address: addr(*address),
                        topics: topics.iter().map(|t| topic(*t)).collect(),
                        data: Default::default(),
                    })
                    .collect();
                if *succeeded {
                    logged.extend(logs.iter().cloned());
                }
                receipts.push(Receipt::new(TxType::Legacy, *succeeded, 21000, logs));
            }
            let header = BlockHeader {
                number,
                timestamp: number * 12,
                logs_bloom: bloom_from_logs(&logged, &NativeCrypto),
                ..Default::default()
            };
            let block = Block::new(
                header,
                BlockBody {
                    transactions,
                    ommers: Default::default(),
                    withdrawals: Default::default(),
                },
            );
            let hash = block.hash();
            storage.add_block(block).await.unwrap();
            storage.add_receipts(hash, receipts).await.unwrap();
            canonical.push((number, hash));
        }
        let (head_number, head_hash) = *canonical.last().unwrap();
        storage
            .forkchoice_update(canonical, head_number, head_hash, None, None)
            .await
            .unwrap();
        storage
    }

    /// Where a returned log sits: `(block number, transaction index, log index, address)`.
    type LogPosition = (u64, u64, u64, u64);

    /// The positions of the logs answering `filter` (a JSON filter object).
    async fn query(storage: &Store, filter: Value) -> Vec<LogPosition> {
        let filter = LogsFilter::parse(&Some(vec![filter])).unwrap();
        let logs = fetch_logs_with_filter(&filter, storage.clone(), &LogQueryLimits::default())
            .await
            .unwrap();
        for log in &logs {
            let header = storage.get_block_header(log.block_number).unwrap().unwrap();
            assert_eq!(log.block_hash, header.hash());
            assert_eq!(log.block_timestamp, header.timestamp);
            assert!(!log.removed);
        }
        logs.iter()
            .map(|log| {
                (
                    log.block_number,
                    log.transaction_index,
                    log.log_index,
                    log.log.address.to_low_u64_be(),
                )
            })
            .collect()
    }

    /// Pins which logs each kind of filter returns, and in what order, over a small chain
    /// with several addresses, topics, a failed receipt and empty blocks. Each block has at
    /// most one transaction: the in-memory backend's receipt iterator only returns a
    /// block's first receipt.
    #[tokio::test]
    async fn get_logs_returns_the_matching_logs_in_order() {
        let storage = store_with_blocks(&[
            vec![],
            vec![(true, vec![(1, vec![1]), (2, vec![1, 2])])],
            vec![(false, vec![(1, vec![1])])],
            vec![(true, vec![(3, vec![3])])],
            vec![(true, vec![(2, vec![2, 1])])],
            vec![],
            vec![(true, vec![(1, vec![1, 2, 3]), (1, vec![])])],
        ])
        .await;
        let t = |n: u64| format!("{:#x}", topic(n));
        let a = |n: u64| format!("{:#x}", addr(n));
        let block_1_hash = format!(
            "{:#x}",
            storage.get_block_header(1).unwrap().unwrap().hash()
        );
        let range = |filter: Value| {
            let mut filter = filter;
            filter["fromBlock"] = json!("0x0");
            filter["toBlock"] = json!("0x6");
            filter
        };
        let with_a_topic = vec![
            (1, 0, 0, 1),
            (1, 0, 1, 2),
            (3, 0, 0, 3),
            (4, 0, 0, 2),
            (6, 0, 0, 1),
        ];

        let cases: Vec<(&str, Value, Vec<LogPosition>)> = vec![
            (
                "no filter",
                range(json!({})),
                vec![
                    (1, 0, 0, 1),
                    (1, 0, 1, 2),
                    (3, 0, 0, 3),
                    (4, 0, 0, 2),
                    (6, 0, 0, 1),
                    (6, 0, 1, 1),
                ],
            ),
            (
                "one address",
                range(json!({"address": a(1)})),
                vec![(1, 0, 0, 1), (6, 0, 0, 1), (6, 0, 1, 1)],
            ),
            (
                "address list",
                range(json!({"address": [a(2), a(3)]})),
                vec![(1, 0, 1, 2), (3, 0, 0, 3), (4, 0, 0, 2)],
            ),
            (
                "topic0",
                range(json!({"topics": [t(1)]})),
                vec![(1, 0, 0, 1), (1, 0, 1, 2), (6, 0, 0, 1)],
            ),
            (
                "topic1 after a wildcard",
                range(json!({"topics": [null, t(1)]})),
                vec![(4, 0, 0, 2)],
            ),
            (
                "OR-set",
                range(json!({"topics": [[t(2), t(3)]]})),
                vec![(3, 0, 0, 3), (4, 0, 0, 2)],
            ),
            (
                "empty OR-set needs a topic at that position",
                range(json!({"topics": [[]]})),
                with_a_topic.clone(),
            ),
            (
                "OR-set with null is a wildcard",
                range(json!({"topics": [[null, t(9)]]})),
                with_a_topic,
            ),
            (
                "address and two topics",
                range(json!({"address": a(1), "topics": [t(1), t(2)]})),
                vec![(6, 0, 0, 1)],
            ),
            ("no match", range(json!({"topics": [t(9)]})), vec![]),
            (
                "sub-range",
                json!({"fromBlock": "0x3", "toBlock": "0x5"}),
                vec![(3, 0, 0, 3), (4, 0, 0, 2)],
            ),
            (
                "block hash",
                json!({"blockHash": block_1_hash, "address": a(2)}),
                vec![(1, 0, 1, 2)],
            ),
        ];
        for (name, filter, expected) in cases {
            assert_eq!(query(&storage, filter).await, expected, "{name}");
        }
    }

    /// Header reads go in chunks of `HEADER_CHUNK_SIZE` blocks; logs on both sides of each
    /// chunk boundary, and at both ends of the range, are all found, in block order.
    #[tokio::test]
    async fn get_logs_spans_header_chunks() {
        let chunk = HEADER_CHUNK_SIZE as usize;
        let logged = [
            0,
            chunk - 1,
            chunk,
            chunk + 1,
            2 * chunk - 1,
            2 * chunk,
            2 * chunk + 2,
        ];
        let mut blocks = vec![vec![]; 2 * chunk + 3];
        for number in logged {
            blocks[number] = vec![(true, vec![(1, vec![1])])];
        }
        let storage = store_with_blocks(&blocks).await;

        let found: Vec<u64> = query(
            &storage,
            json!({"fromBlock": "0x0", "toBlock": format!("{:#x}", 2 * chunk + 2)}),
        )
        .await
        .into_iter()
        .map(|(block_number, ..)| block_number)
        .collect();
        assert_eq!(found, logged.map(|number| number as u64));

        // A range that starts and ends inside chunks.
        let found: Vec<u64> = query(
            &storage,
            json!({"fromBlock": format!("{:#x}", chunk), "toBlock": format!("{:#x}", 2 * chunk - 1)}),
        )
        .await
        .into_iter()
        .map(|(block_number, ..)| block_number)
        .collect();
        assert_eq!(
            found,
            [chunk, chunk + 1, 2 * chunk - 1].map(|number| number as u64)
        );
    }

    /// Runs `filter` (a JSON filter object) through the log scan under `limits`.
    async fn query_with(
        storage: &Store,
        filter: Value,
        limits: &LogQueryLimits,
    ) -> Result<Vec<RpcLog>, RpcErr> {
        let filter = LogsFilter::parse(&Some(vec![filter])).unwrap();
        fetch_logs_with_filter(&filter, storage.clone(), limits).await
    }

    /// The JSON-RPC code and message a failed query answers with.
    fn code_and_message(result: Result<Vec<RpcLog>, RpcErr>) -> (i32, String) {
        let error: crate::utils::RpcErrorMetadata = result.unwrap_err().into();
        (error.code, error.message)
    }

    fn hex(number: u64) -> String {
        format!("{number:#x}")
    }

    /// `count` distinct addresses as a JSON list, starting at `addr(first)`.
    fn address_list(first: u64, count: u64) -> Value {
        json!(
            (first..first + count)
                .map(|n| format!("{:#x}", addr(n)))
                .collect::<Vec<_>>()
        )
    }

    /// Limits with only `max_log_query_work` set and the given work budget.
    fn work_limits(max_log_query_work: u64, work_budget: u64) -> LogQueryLimits {
        LogQueryLimits::new(0, 0, max_log_query_work, work_budget)
    }

    /// `blocks + 1` blocks, each with one log from `addr(1)` with `topic(1)`, except the
    /// genesis block.
    async fn store_with_one_log_per_block(blocks: usize) -> Store {
        let mut chain = vec![vec![(true, vec![(1, vec![1])])]; blocks + 1];
        chain[0] = vec![];
        store_with_blocks(&chain).await
    }

    #[tokio::test]
    async fn get_logs_rejects_ranges_over_the_block_limit() {
        let storage = store_with_one_log_per_block(15).await;
        let limits = LogQueryLimits::new(10, 0, 0, DEFAULT_LOG_QUERY_WORK_BUDGET);

        // `to - from` is the range, as in reth and geth: 11 blocks are within a limit of 10.
        let logs = query_with(
            &storage,
            json!({"fromBlock": "0x0", "toBlock": hex(10)}),
            &limits,
        )
        .await
        .unwrap();
        assert_eq!(logs.len(), 10);

        let result = query_with(
            &storage,
            json!({"fromBlock": "0x0", "toBlock": hex(11)}),
            &limits,
        )
        .await;
        assert_eq!(
            code_and_message(result),
            (-32602, "query exceeds max block range 10".to_string())
        );

        // The existing range errors keep their precedence.
        let result = query_with(
            &storage,
            json!({"fromBlock": hex(12), "toBlock": "0x0"}),
            &limits,
        )
        .await;
        assert_eq!(
            code_and_message(result),
            (-32602, "invalid block range params".to_string())
        );
    }

    #[tokio::test]
    async fn get_logs_rejects_queries_over_the_work_limit() {
        let storage = store_with_one_log_per_block(15).await;
        let limits = work_limits(100, DEFAULT_LOG_QUERY_WORK_BUDGET);
        let range = |filter: Value| {
            let mut filter = filter;
            filter["fromBlock"] = json!("0x0");
            filter["toBlock"] = json!(hex(14));
            filter
        };
        let topics = [topic(1), topic(2)].map(|t| format!("{t:#x}"));

        // 15 blocks x 3 addresses x 2 alternatives = 90.
        let within = range(json!({"address": address_list(1, 3), "topics": [topics]}));
        assert_eq!(
            query_with(&storage, within, &limits).await.unwrap().len(),
            14
        );

        // 15 blocks x 4 addresses x 2 alternatives = 120.
        let over = range(json!({"address": address_list(1, 4), "topics": [topics]}));
        assert_eq!(
            code_and_message(query_with(&storage, over, &limits).await),
            (
                -32005,
                "log query too expensive (blocks 15 x addresses 4 x topic alternatives 2 = 120 units of work, limit 100); narrow the block range or the filter".to_string()
            )
        );

        // Only the largest OR-set counts, and one containing `null` is a wildcard:
        // 15 x 1 x 6 = 90, where multiplying the positions would give 15 x 2 x 6 = 180.
        let six = (1..=6)
            .map(|n| format!("{:#x}", topic(n)))
            .collect::<Vec<_>>();
        let wildcard = json!([null, format!("{:#x}", topic(7))]);
        let positions = range(json!({"topics": [topics, six, wildcard]}));
        assert!(query_with(&storage, positions, &limits).await.is_ok());

        // A blockHash query counts one block.
        let block_hash = format!(
            "{:#x}",
            storage.get_block_header(1).unwrap().unwrap().hash()
        );
        let by_hash = json!({"blockHash": block_hash, "address": address_list(1, 100)});
        assert!(query_with(&storage, by_hash, &limits).await.is_ok());
        let by_hash = json!({"blockHash": block_hash, "address": address_list(1, 101)});
        assert_eq!(
            code_and_message(query_with(&storage, by_hash, &limits).await).0,
            -32005
        );
    }

    #[tokio::test]
    async fn get_logs_rejects_responses_over_the_log_limit() {
        // Blocks 1 to 3 have one log each, block 4 has three.
        let one_log = vec![(true, vec![(1, vec![1])])];
        let storage = store_with_blocks(&[
            vec![],
            one_log.clone(),
            one_log.clone(),
            one_log,
            vec![(true, vec![(1, vec![1]), (1, vec![1]), (1, vec![1])])],
            vec![],
        ])
        .await;
        let limits = LogQueryLimits::new(0, 2, 0, DEFAULT_LOG_QUERY_WORK_BUDGET);
        let range = |from: u64, to: u64| json!({"fromBlock": hex(from), "toBlock": hex(to)});

        assert_eq!(
            query_with(&storage, range(1, 2), &limits)
                .await
                .unwrap()
                .len(),
            2
        );
        // The third log arrives with block 3, so blocks 0 to 2 are the ones to retry.
        assert_eq!(
            code_and_message(query_with(&storage, range(0, 3), &limits).await),
            (
                -32602,
                "query exceeds max results 2, retry with the range 0-2".to_string()
            )
        );
        // The first block already goes over: the suggestion is that block alone.
        assert_eq!(
            code_and_message(query_with(&storage, range(4, 5), &limits).await).1,
            "query exceeds max results 2, retry with the range 4-4"
        );
        // A single block cannot be split, so it is exempt, by number or by hash.
        assert_eq!(
            query_with(&storage, range(4, 4), &limits)
                .await
                .unwrap()
                .len(),
            3
        );
        let block_hash = format!(
            "{:#x}",
            storage.get_block_header(4).unwrap().unwrap().hash()
        );
        let by_hash = json!({"blockHash": block_hash});
        assert_eq!(
            query_with(&storage, by_hash, &limits).await.unwrap().len(),
            3
        );
    }

    /// Heavy queries fail at once while the budget is spent, light ones never touch it,
    /// and the budget is whole again after a query succeeds or fails.
    #[tokio::test]
    async fn get_logs_work_budget_is_held_while_running_and_released_after() {
        let storage = store_with_one_log_per_block(15).await;
        let budget = 100_000;
        let limits = LogQueryLimits::new(0, 1, 0, budget);
        // 16 blocks x 700 addresses = 11,200 units, above the light threshold.
        let heavy =
            json!({"fromBlock": "0x0", "toBlock": hex(15), "address": address_list(1, 700)});
        let light = json!({"fromBlock": "0x1", "toBlock": "0x1"});

        let held = limits
            .work_budget
            .clone()
            .try_acquire_many_owned(budget as u32)
            .unwrap();
        assert_eq!(
            code_and_message(query_with(&storage, heavy.clone(), &limits).await),
            (
                -32005,
                "too many log queries in flight, retry later".to_string()
            )
        );
        assert_eq!(query_with(&storage, light, &limits).await.unwrap().len(), 1);
        drop(held);

        // Fails on the log limit after reserving its work, and gives it back.
        assert_eq!(
            code_and_message(query_with(&storage, heavy.clone(), &limits).await).0,
            -32602
        );
        assert_eq!(limits.work_budget.available_permits(), budget as usize);

        let unlimited_logs = LogQueryLimits::new(0, 0, 0, budget);
        assert_eq!(
            query_with(&storage, heavy, &unlimited_logs)
                .await
                .unwrap()
                .len(),
            15
        );
        assert_eq!(
            unlimited_logs.work_budget.available_permits(),
            budget as usize
        );
    }

    /// A query dropped mid-scan (the client went away) gives its work back.
    #[tokio::test]
    async fn get_logs_work_budget_is_released_when_a_query_is_dropped() {
        let storage = store_with_one_log_per_block(1_500).await;
        let budget = 100_000;
        let limits = work_limits(0, budget);
        let filter = LogsFilter::parse(&Some(vec![json!({
            "fromBlock": "0x0",
            "toBlock": hex(1_500),
            "address": address_list(1, 10),
        })]))
        .unwrap();

        let mut scan = Box::pin(fetch_logs_with_filter(&filter, storage.clone(), &limits));
        // Poll once: the query reserves its work and waits on its first header chunk.
        let first_poll = tokio::time::timeout(std::time::Duration::ZERO, &mut scan).await;
        assert!(first_poll.is_err(), "the scan should still be running");
        assert!(limits.work_budget.available_permits() < budget as usize);

        drop(scan);
        assert_eq!(limits.work_budget.available_permits(), budget as usize);
    }

    /// `eth_getFilterChanges` scans through the same path, so it returns what
    /// `eth_getLogs` returns for the same range and gets the same limit errors, and
    /// `eth_newFilter` refuses a filter that could not scan even one block.
    #[tokio::test]
    async fn filter_changes_match_get_logs_and_share_its_limits() {
        use crate::eth::filter::{FilterKind, PollableFilter};
        use crate::rpc::map_http_requests;
        use crate::test_utils::default_context_with_storage;
        use crate::utils::RpcRequest;
        use std::time::Instant;

        let storage = store_with_one_log_per_block(5).await;
        let mut context = default_context_with_storage(storage.clone()).await;
        let call = |method: &str, params: Value| {
            serde_json::from_value::<RpcRequest>(
                json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
            )
            .unwrap()
        };
        let watch_from_genesis = |context: &crate::rpc::RpcApiContext| {
            let filter =
                LogsFilter::parse(&Some(vec![json!({"address": format!("{:#x}", addr(1))})]))
                    .unwrap();
            context.active_filters.lock().unwrap().insert(
                7,
                (
                    Instant::now(),
                    PollableFilter {
                        last_block_number: 0,
                        kind: FilterKind::Logs(filter),
                    },
                ),
            );
        };

        watch_from_genesis(&context);
        let changes = map_http_requests(
            &call("eth_getFilterChanges", json!(["0x7"])),
            context.clone(),
        )
        .await
        .unwrap();
        let logs = map_http_requests(
            &call(
                "eth_getLogs",
                json!([{"fromBlock": "0x0", "toBlock": "0x5", "address": format!("{:#x}", addr(1))}]),
            ),
            context.clone(),
        )
        .await
        .unwrap();
        assert_eq!(changes, logs);
        assert_eq!(changes.as_array().unwrap().len(), 5);

        context.log_query_limits = LogQueryLimits::new(0, 2, 0, DEFAULT_LOG_QUERY_WORK_BUDGET);
        watch_from_genesis(&context);
        let error: crate::utils::RpcErrorMetadata = map_http_requests(
            &call("eth_getFilterChanges", json!(["0x7"])),
            context.clone(),
        )
        .await
        .unwrap_err()
        .into();
        assert_eq!(
            (error.code, error.message.as_str()),
            (
                -32602,
                "query exceeds max results 2, retry with the range 0-2"
            )
        );

        context.log_query_limits = work_limits(10, DEFAULT_LOG_QUERY_WORK_BUDGET);
        let error: crate::utils::RpcErrorMetadata = map_http_requests(
            &call("eth_newFilter", json!([{"address": address_list(1, 11)}])),
            context.clone(),
        )
        .await
        .unwrap_err()
        .into();
        assert_eq!(error.code, -32005);
        let accepted = map_http_requests(
            &call("eth_newFilter", json!([{"address": address_list(1, 10)}])),
            context.clone(),
        )
        .await;
        assert!(accepted.is_ok(), "{accepted:?}");
    }
}
