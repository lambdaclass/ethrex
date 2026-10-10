// The behaviour of the filtering endpoints is based on:
// - Manually testing the behaviour deploying contracts on the Sepolia test network.
// - Go-Ethereum, specifically: https://github.com/ethereum/go-ethereum/blob/368e16f39d6c7e5cce72a92ec289adbfbaed4854/eth/filters/filter.go
// - Ethereum's reference: https://ethereum.org/en/developers/docs/apis/json-rpc/#eth_newfilter
use ethrex_common::types::BlockNumber;
use ethrex_storage::Store;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tracing::error;

use crate::rpc::RpcHandler;
use crate::{
    types::block_identifier::{BlockIdentifier, BlockTag},
    utils::{RpcErr, RpcRequest, parse_json_hex},
};
use serde_json::{Value, json};

use super::logs::{LogsFilter, fetch_logs_with_filter};

#[derive(Debug, Clone)]
pub struct NewFilterRequest {
    pub request_data: LogsFilter,
}

/// Used by the tokio runtime to clean outdated filters
/// Takes 2 arguments:
/// - filters: the filters to clean up.
/// - filter_duration: represents how many *seconds* filter can last,
///   if any filter is older than this, it will be removed.
pub fn clean_outdated_filters(filters: ActiveFilters, filter_duration: Duration) {
    let mut active_filters_guard = filters.lock().unwrap_or_else(|mut poisoned_guard| {
        error!("THREAD CRASHED WITH MUTEX TAKEN; SYSTEM MIGHT BE UNSTABLE");
        **poisoned_guard.get_mut() = HashMap::new();
        filters.clear_poison();
        poisoned_guard.into_inner()
    });

    // Keep only filters that have not expired.
    active_filters_guard
        .retain(|_, (filter_timestamp, _)| filter_timestamp.elapsed() <= filter_duration);
}
/// Maps IDs to active pollable filters and their timestamps.
pub type ActiveFilters = Arc<Mutex<HashMap<u64, (Instant, PollableFilter)>>>;

/// What a pollable filter yields on each `eth_getFilterChanges`.
#[derive(Debug, Clone)]
pub enum FilterKind {
    /// `eth_newFilter`: logs matching the filter over the polled range.
    Logs(LogsFilter),
    /// `eth_newBlockFilter`: the hashes of blocks added since the last poll.
    Blocks,
}

#[derive(Debug, Clone)]
pub struct PollableFilter {
    /// Last block number from when this
    /// filter was requested or created.
    /// i.e. if this filter is requested,
    /// the log will be applied from this
    /// block number up to the latest one.
    pub last_block_number: BlockNumber,
    pub kind: FilterKind,
}

impl NewFilterRequest {
    pub fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, RpcErr> {
        let filter = LogsFilter::parse(params)?;
        // EIP-234 defines `blockHash` for eth_newFilter too, but the only thing
        // it does with such a filter is answer eth_getFilterLogs, which we don't
        // implement. For eth_getFilterChanges, a moving range by definition, even
        // geth ignores the hash. Reject it instead of accepting and ignoring.
        if filter.block_hash.is_some() {
            return Err(RpcErr::BadParams(
                "`blockHash` is not a valid filter for eth_newFilter".to_string(),
            ));
        }
        Ok(NewFilterRequest {
            request_data: filter,
        })
    }

    pub async fn handle(
        &self,
        storage: ethrex_storage::Store,
        filters: ActiveFilters,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        let from = self
            .request_data
            .from_block
            .resolve_block_number(&storage)
            .await?
            .ok_or(RpcErr::WrongParam("fromBlock".to_string()))?;
        let to = self
            .request_data
            .to_block
            .resolve_block_number(&storage)
            .await?
            .ok_or(RpcErr::WrongParam("toBlock".to_string()))?;

        if (from..=to).is_empty() {
            return Err(RpcErr::BadParams("Invalid block range".to_string()));
        }

        let last_block_number = storage.get_latest_block_number()?;
        let id: u64 = rand::random();
        let timestamp = Instant::now();
        let mut active_filters_guard = filters.lock().unwrap_or_else(|mut poisoned_guard| {
            error!("THREAD CRASHED WITH MUTEX TAKEN; SYSTEM MIGHT BE UNSTABLE");
            **poisoned_guard.get_mut() = HashMap::new();
            filters.clear_poison();
            poisoned_guard.into_inner()
        });
        active_filters_guard.insert(
            id,
            (
                timestamp,
                PollableFilter {
                    last_block_number,
                    kind: FilterKind::Logs(self.request_data.clone()),
                },
            ),
        );
        let as_hex = json!(format!("0x{:x}", id));
        Ok(as_hex)
    }

    pub async fn stateful_call(
        req: &RpcRequest,
        storage: Store,
        state: ActiveFilters,
    ) -> Result<Value, RpcErr> {
        let request = Self::parse(&req.params)?;
        request.handle(storage, state).await
    }
}

/// `eth_newBlockFilter`: registers a filter that yields the hash of every block
/// appended since the previous poll. Takes no parameters.
pub struct NewBlockFilterRequest;

impl NewBlockFilterRequest {
    pub fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, RpcErr> {
        // Other clients accept both an absent and an empty params array here.
        match params.as_deref() {
            None | Some([]) => Ok(NewBlockFilterRequest),
            Some(_) => Err(RpcErr::BadParams("Expected no params".to_string())),
        }
    }

    pub async fn handle(
        &self,
        storage: ethrex_storage::Store,
        filters: ActiveFilters,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        // Anchor at the current head so the first poll reports only blocks that
        // arrive after registration, matching the log filters and other clients.
        let last_block_number = storage.get_latest_block_number()?;
        let id: u64 = rand::random();
        let mut active_filters_guard = filters.lock().unwrap_or_else(|mut poisoned_guard| {
            error!("THREAD CRASHED WITH MUTEX TAKEN; SYSTEM MIGHT BE UNSTABLE");
            **poisoned_guard.get_mut() = HashMap::new();
            filters.clear_poison();
            poisoned_guard.into_inner()
        });
        active_filters_guard.insert(
            id,
            (
                Instant::now(),
                PollableFilter {
                    last_block_number,
                    kind: FilterKind::Blocks,
                },
            ),
        );
        Ok(json!(format!("0x{:x}", id)))
    }

    pub async fn stateful_call(
        req: &RpcRequest,
        storage: Store,
        state: ActiveFilters,
    ) -> Result<Value, RpcErr> {
        let request = Self::parse(&req.params)?;
        request.handle(storage, state).await
    }
}

pub struct DeleteFilterRequest {
    pub id: u64,
}

impl DeleteFilterRequest {
    pub fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, RpcErr> {
        match params.as_deref() {
            Some([param]) => {
                let id = parse_json_hex(param).map_err(|_err| RpcErr::BadHexFormat(0))?;
                Ok(DeleteFilterRequest { id })
            }
            Some(_) => Err(RpcErr::BadParams(
                "Expected an array with a single hex encoded id".to_string(),
            )),
            None => Err(RpcErr::MissingParam("0".to_string())),
        }
    }

    pub fn handle(
        &self,
        _storage: ethrex_storage::Store,
        filters: ActiveFilters,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        let mut active_filters_guard = filters.lock().unwrap_or_else(|mut poisoned_guard| {
            error!("THREAD CRASHED WITH MUTEX TAKEN; SYSTEM MIGHT BE UNSTABLE");
            **poisoned_guard.get_mut() = HashMap::new();
            filters.clear_poison();
            poisoned_guard.into_inner()
        });
        match active_filters_guard.remove(&self.id) {
            Some(_) => Ok(true.into()),
            None => Ok(false.into()),
        }
    }

    pub fn stateful_call(
        req: &RpcRequest,
        storage: ethrex_storage::Store,
        filters: ActiveFilters,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        let request = Self::parse(&req.params)?;
        request.handle(storage, filters)
    }
}

pub struct FilterChangesRequest {
    pub id: u64,
}

impl FilterChangesRequest {
    pub fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, RpcErr> {
        match params.as_deref() {
            Some([param]) => {
                let id = parse_json_hex(param).map_err(|_err| RpcErr::BadHexFormat(0))?;
                Ok(FilterChangesRequest { id })
            }
            Some(_) => Err(RpcErr::BadParams(
                "Expected an array with a single hex encoded id".to_string(),
            )),
            None => Err(RpcErr::MissingParam("0".to_string())),
        }
    }
    pub async fn handle(
        &self,
        storage: ethrex_storage::Store,
        filters: ActiveFilters,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        let latest_block_num = storage.get_latest_block_number()?;
        // Box needed to keep the future Sync
        // https://github.com/rust-lang/rust/issues/128095
        let mut active_filters_guard =
            Box::new(filters.lock().unwrap_or_else(|mut poisoned_guard| {
                error!("THREAD CRASHED WITH MUTEX TAKEN; SYSTEM MIGHT BE UNSTABLE");
                **poisoned_guard.get_mut() = HashMap::new();
                filters.clear_poison();
                poisoned_guard.into_inner()
            }));
        if let Some((timestamp, filter)) = active_filters_guard.get_mut(&self.id) {
            // A block filter has no range to validate: it always reports whatever
            // blocks arrived since the last poll.
            if matches!(filter.kind, FilterKind::Blocks) {
                *timestamp = Instant::now();
                let from = filter.last_block_number.saturating_add(1);
                filter.last_block_number = latest_block_num;
                drop(active_filters_guard);
                let mut hashes = Vec::new();
                for number in from..=latest_block_num {
                    // A number with no canonical hash means the chain reorged out
                    // from under us mid-scan; skip rather than fail the poll.
                    if let Some(hash) = storage.get_canonical_block_hash(number).await? {
                        hashes.push(format!("{hash:#x}"));
                    }
                }
                return serde_json::to_value(hashes).map_err(|error| {
                    tracing::error!("Block filter request failed with: {error}");
                    RpcErr::Internal("Failed to collect block hashes".to_string())
                });
            }
            let FilterKind::Logs(filter_data) = &filter.kind else {
                unreachable!("block filters are handled above")
            };
            // A poll reports the blocks added since the previous poll (on the first one,
            // since the filter was created), up to the filter's `toBlock` when that is a
            // number. A filter whose `toBlock` is a tag other than `latest`, or a number
            // already reported, has nothing left to report.
            let from = filter.last_block_number.saturating_add(1);
            let to = match filter_data.to_block {
                BlockIdentifier::Tag(BlockTag::Latest) => latest_block_num,
                BlockIdentifier::Number(block_num) if block_num >= from => {
                    block_num.min(latest_block_num)
                }
                _ => return Ok(json!([])),
            };
            // Since the filter was polled, updated its timestamp, so
            // it does not expire.
            *timestamp = Instant::now();
            if from > to {
                // No block was added since the last poll.
                return Ok(json!([]));
            }
            // The stored filter keeps its own range; only the poll position moves.
            let mut logs_filter = filter_data.clone();
            logs_filter.from_block = BlockIdentifier::Number(from);
            logs_filter.to_block = BlockIdentifier::Number(to);
            // Drop the lock early to process this filter's query
            // and not keep the lock more than we should.
            drop(active_filters_guard);
            let logs = fetch_logs_with_filter(&logs_filter, storage).await?;
            // Move the poll position only once the logs are in hand, so a poll that fails,
            // or whose client goes away mid-scan, leaves its blocks for the next poll. Two
            // polls of the same filter running at once can both report them.
            let mut active_filters_guard = filters.lock().unwrap_or_else(|mut poisoned_guard| {
                error!("THREAD CRASHED WITH MUTEX TAKEN; SYSTEM MIGHT BE UNSTABLE");
                **poisoned_guard.get_mut() = HashMap::new();
                filters.clear_poison();
                poisoned_guard.into_inner()
            });
            if let Some((_, filter)) = active_filters_guard.get_mut(&self.id) {
                filter.last_block_number = filter.last_block_number.max(to);
            }
            drop(active_filters_guard);
            serde_json::to_value(logs).map_err(|error| {
                tracing::error!("Log filtering request failed with: {error}");
                RpcErr::Internal("Failed to filter logs".to_string())
            })
        } else {
            Err(RpcErr::BadParams(
                "No matching filter for given id".to_string(),
            ))
        }
    }
    pub async fn stateful_call(
        req: &RpcRequest,
        storage: ethrex_storage::Store,
        filters: ActiveFilters,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        let request = Self::parse(&req.params)?;
        request.handle(storage, filters).await
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    use super::ActiveFilters;
    use crate::{
        eth::{
            filter::PollableFilter,
            logs::{AddressFilter, LogsFilter, TopicFilter},
        },
        rpc::{FILTER_DURATION, map_http_requests},
        test_utils::{TEST_GENESIS, default_context_with_storage, start_test_api},
    };
    use crate::{types::block_identifier::BlockIdentifier, utils::RpcRequest};
    use ethrex_common::types::Genesis;
    use ethrex_storage::{EngineType, Store};

    use serde_json::{Value, json};

    /// Every filter these tests register is a log filter; unwrap the kind so the
    /// assertions can keep reading the `LogsFilter` fields directly.
    fn logs_filter(filter: &PollableFilter) -> &LogsFilter {
        match &filter.kind {
            crate::eth::filter::FilterKind::Logs(data) => data,
            crate::eth::filter::FilterKind::Blocks => {
                panic!("test filter should be a log filter")
            }
        }
    }

    #[tokio::test]
    async fn filter_request_smoke_test_valid_params() {
        let filter_req_params = json!(
                {
                    "fromBlock": "0x1",
                    "toBlock": "0x2",
                    "address": null,
                    "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"]
                }
        );
        let raw_json = json!(
        {
            "jsonrpc":"2.0",
            "method":"eth_newFilter",
            "params":
            [
                filter_req_params.clone()
            ]
                ,"id":1
        });
        let filters = Arc::new(Mutex::new(HashMap::new()));
        let id = run_new_filter_request_test(raw_json.clone(), filters.clone()).await;
        let filters = filters.lock().unwrap();
        assert!(filters.len() == 1);
        let (_, filter) = filters.clone().get(&id).unwrap().clone();
        assert!(matches!(
            logs_filter(&filter).from_block,
            BlockIdentifier::Number(1)
        ));
        assert!(matches!(
            logs_filter(&filter).to_block,
            BlockIdentifier::Number(2)
        ));
        assert!(logs_filter(&filter).address_filters.is_none());
        assert!(matches!(
            &logs_filter(&filter).topics[..],
            [TopicFilter::Topic(_)]
        ));
    }

    #[tokio::test]
    async fn filter_request_smoke_test_valid_null_topics_null_addr() {
        let raw_json = json!(
        {
            "jsonrpc":"2.0",
            "method":"eth_newFilter",
            "params":
            [
                {
                    "fromBlock": "0x1",
                    "toBlock": "0xFF",
                    "topics": null,
                    "address": null
                }
            ]
                ,"id":1
        });
        let filters = Arc::new(Mutex::new(HashMap::new()));
        let id = run_new_filter_request_test(raw_json.clone(), filters.clone()).await;
        let filters = filters.lock().unwrap();
        assert!(filters.len() == 1);
        let (_, filter) = filters.clone().get(&id).unwrap().clone();
        assert!(matches!(
            logs_filter(&filter).from_block,
            BlockIdentifier::Number(1)
        ));
        assert!(matches!(
            logs_filter(&filter).to_block,
            BlockIdentifier::Number(255)
        ));
        assert!(logs_filter(&filter).address_filters.is_none());
        assert!(matches!(&logs_filter(&filter).topics[..], []));
    }

    #[tokio::test]
    async fn filter_request_smoke_test_valid_addr_topic_null() {
        let raw_json = json!(
        {
            "jsonrpc":"2.0",
            "method":"eth_newFilter",
            "params":
            [
                {
                    "fromBlock": "0x1",
                    "toBlock": "0xFF",
                    "topics": null,
                    "address": [ "0xb794f5ea0ba39494ce839613fffba74279579268" ]
                }
            ]
                ,"id":1
        });
        let filters = Arc::new(Mutex::new(HashMap::new()));
        let id = run_new_filter_request_test(raw_json.clone(), filters.clone()).await;
        let filters = filters.lock().unwrap();
        assert!(filters.len() == 1);
        let (_, filter) = filters.clone().get(&id).unwrap().clone();
        assert!(matches!(
            logs_filter(&filter).from_block,
            BlockIdentifier::Number(1)
        ));
        assert!(matches!(
            logs_filter(&filter).to_block,
            BlockIdentifier::Number(255)
        ));
        assert!(matches!(
            logs_filter(&filter).address_filters.clone().unwrap(),
            AddressFilter::Many(_)
        ));
        assert!(matches!(&logs_filter(&filter).topics[..], []));
    }

    #[tokio::test]
    #[should_panic]
    async fn filter_request_smoke_test_invalid_block_range() {
        let raw_json = json!(
        {
            "jsonrpc":"2.0",
            "method":"eth_newFilter",
            "params":
            [
                {
                    "fromBlock": "0xFFF",
                    "toBlock": "0xA",
                    "topics": null,
                    "address": null
                }
            ]
                ,"id":1
        });
        run_new_filter_request_test(raw_json.clone(), Default::default()).await;
    }

    #[tokio::test]
    #[should_panic]
    async fn filter_request_smoke_test_from_block_missing() {
        let raw_json = json!(
        {
            "jsonrpc":"2.0",
            "method":"eth_newFilter",
            "params":
            [
                {
                    "fromBlock": null,
                    "toBlock": "0xA",
                    "topics": null,
                    "address": null
                }
            ]
                ,"id":1
        });
        let filters = Arc::new(Mutex::new(HashMap::new()));
        run_new_filter_request_test(raw_json.clone(), filters.clone()).await;
    }

    async fn run_new_filter_request_test(
        json_req: serde_json::Value,
        filters_pointer: ActiveFilters,
    ) -> u64 {
        let storage = Store::new("in-mem", EngineType::InMemory)
            .expect("Fatal: could not create in memory test db");
        let mut context = default_context_with_storage(storage).await;
        context.active_filters = filters_pointer.clone();

        let request: RpcRequest = serde_json::from_value(json_req).expect("Test json is incorrect");
        let genesis_config: Genesis =
            serde_json::from_str(TEST_GENESIS).expect("Fatal: non-valid genesis test config");

        context
            .storage
            .add_initial_state(genesis_config)
            .await
            .expect("Fatal: could not add test genesis in test");
        let response = map_http_requests(&request, context.clone())
            .await
            .unwrap()
            .to_string();
        let trimmed_id = response.trim().trim_matches('"');
        assert!(trimmed_id.starts_with("0x"));
        let hex = trimmed_id.trim_start_matches("0x");
        let parsed = u64::from_str_radix(hex, 16);
        assert!(u64::from_str_radix(hex, 16).is_ok());
        parsed.unwrap()
    }

    #[tokio::test]
    async fn install_filter_removed_correctly_test() {
        let uninstall_filter_req: RpcRequest = serde_json::from_value(json!(
        {
            "jsonrpc":"2.0",
            "method":"eth_uninstallFilter",
            "params":
            [
                "0xFF"
            ]
                ,"id":1
        }))
        .expect("Json for test is not a valid request");
        let filter = (
            0xFF,
            (
                Instant::now(),
                PollableFilter {
                    last_block_number: 0,
                    kind: crate::eth::filter::FilterKind::Logs(LogsFilter {
                        from_block: BlockIdentifier::Number(1),
                        to_block: BlockIdentifier::Number(2),
                        block_hash: None,
                        address_filters: None,
                        topics: vec![],
                    }),
                },
            ),
        );
        let active_filters = Arc::new(Mutex::new(HashMap::from([filter])));

        let storage = Store::new("in-mem", EngineType::InMemory)
            .expect("Fatal: could not create in memory test db");

        let mut context = default_context_with_storage(storage).await;
        context.active_filters = active_filters.clone();

        map_http_requests(&uninstall_filter_req, context.clone())
            .await
            .unwrap();

        assert!(
            active_filters.clone().lock().unwrap().is_empty(),
            "Expected filter map to be empty after request"
        );
    }

    #[tokio::test]
    async fn removing_non_existing_filter_returns_false() {
        let active_filters = Arc::new(Mutex::new(HashMap::new()));

        let storage = Store::new("in-mem", EngineType::InMemory)
            .expect("Fatal: could not create in memory test db");
        let mut context = default_context_with_storage(storage).await;
        context.active_filters = active_filters.clone();

        let uninstall_filter_req: RpcRequest = serde_json::from_value(json!(
        {
            "jsonrpc":"2.0",
            "method":"eth_uninstallFilter",
            "params":
            [
                "0xFF"
            ]
                ,"id":1
        }))
        .expect("Json for test is not a valid request");
        let res = map_http_requests(&uninstall_filter_req, context.clone())
            .await
            .unwrap();
        assert!(matches!(res, serde_json::Value::Bool(false)));
    }

    #[tokio::test]
    async fn background_job_removes_filter_smoke_test() {
        // Start a test server to start the cleanup
        // task in the background
        let server_handle = start_test_api().await;

        // Give the server some time to start
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Install a filter through the endpiont
        let client = reqwest::Client::new();
        let raw_json = json!(
        {
            "jsonrpc":"2.0",
            "method":"eth_newFilter",
            "params":
            [
                {
                    "fromBlock": "0x1",
                    "toBlock": "0xA",
                    "topics": null,
                    "address": null
                }
            ]
                ,"id":1
        });
        let response: Value = client
            .post("http://localhost:8500")
            .json(&raw_json)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert!(
            response.get("result").is_some(),
            "Response should have a 'result' field"
        );

        let raw_json = json!(
        {
            "jsonrpc":"2.0",
            "method":"eth_uninstallFilter",
            "params":
            [
                response.get("result").unwrap()
            ]
                ,"id":1
        });

        tokio::time::sleep(FILTER_DURATION).await;
        tokio::time::sleep(FILTER_DURATION).await;

        let response: serde_json::Value = client
            .post("http://localhost:8500")
            .json(&raw_json)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert!(
            matches!(
                response.get("result").unwrap(),
                serde_json::Value::Bool(false)
            ),
            "Filter was expected to be deleted by background job, but it still exists"
        );

        server_handle.abort();
    }

    /// Block `number` with one transaction whose receipt logs once from the address
    /// `log_from`, and that receipt; block 0 gets no transaction.
    fn head_block(
        number: u64,
        log_from: u64,
    ) -> (
        ethrex_common::types::Block,
        Vec<ethrex_common::types::Receipt>,
    ) {
        use ethrex_common::{
            H160,
            types::{
                Block, BlockBody, BlockHeader, LegacyTransaction, Log, Receipt, Transaction,
                TxType, bloom_from_logs,
            },
        };
        use ethrex_crypto::NativeCrypto;

        let logs = vec![Log {
            address: H160::from_low_u64_be(log_from),
            topics: vec![],
            data: Default::default(),
        }];
        let (transactions, receipts, bloom) = if number == 0 {
            (vec![], vec![], Default::default())
        } else {
            (
                vec![Transaction::LegacyTransaction(LegacyTransaction {
                    nonce: number,
                    ..Default::default()
                })],
                vec![Receipt::new(TxType::Legacy, true, 21000, logs.clone())],
                bloom_from_logs(&logs, &NativeCrypto),
            )
        };
        let block = Block::new(
            BlockHeader {
                number,
                logs_bloom: bloom,
                ..Default::default()
            },
            BlockBody {
                transactions,
                ommers: Default::default(),
                withdrawals: Default::default(),
            },
        );
        (block, receipts)
    }

    /// Adds `block` as the new head, without its receipts.
    async fn add_head_without_receipts(
        storage: &Store,
        block: ethrex_common::types::Block,
    ) -> ethrex_common::H256 {
        let (number, hash) = (block.header.number, block.hash());
        storage.add_block(block).await.unwrap();
        storage
            .forkchoice_update(vec![(number, hash)], number, hash, None, None)
            .await
            .unwrap();
        hash
    }

    /// Adds block `number` as the new head, with one transaction whose receipt logs once
    /// from the address `log_from`; block 0 gets no transaction.
    async fn add_head_block(storage: &Store, number: u64, log_from: u64) {
        let (block, receipts) = head_block(number, log_from);
        let hash = add_head_without_receipts(storage, block).await;
        storage.add_receipts(hash, receipts).await.unwrap();
    }

    fn rpc_call(method: &str, params: Value) -> RpcRequest {
        serde_json::from_value(
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
        )
        .unwrap()
    }

    /// The block numbers of the logs one `eth_getFilterChanges` poll returns.
    async fn poll_log_blocks(context: &crate::rpc::RpcApiContext, id: &Value) -> Vec<u64> {
        let changes = map_http_requests(
            &rpc_call("eth_getFilterChanges", json!([id])),
            context.clone(),
        )
        .await
        .unwrap();
        changes
            .as_array()
            .unwrap()
            .iter()
            .map(|log| {
                u64::from_str_radix(
                    log["blockNumber"]
                        .as_str()
                        .unwrap()
                        .trim_start_matches("0x"),
                    16,
                )
                .unwrap()
            })
            .collect()
    }

    /// Each poll returns the logs of the blocks added since the previous one, once: not
    /// the logs of the block that was the head when the filter was created, not the last
    /// polled block again, and still after the chain has moved on.
    #[tokio::test]
    async fn filter_changes_return_each_new_log_once() {
        let storage = Store::new("in-mem", EngineType::InMemory).unwrap();
        add_head_block(&storage, 0, 1).await;
        add_head_block(&storage, 1, 1).await;
        let context = default_context_with_storage(storage.clone()).await;
        let address = format!("{:#x}", ethrex_common::H160::from_low_u64_be(1));
        let id = map_http_requests(
            &rpc_call("eth_newFilter", json!([{"address": address}])),
            context.clone(),
        )
        .await
        .unwrap();

        assert_eq!(poll_log_blocks(&context, &id).await, Vec::<u64>::new());
        add_head_block(&storage, 2, 1).await;
        assert_eq!(poll_log_blocks(&context, &id).await, vec![2]);
        assert_eq!(poll_log_blocks(&context, &id).await, Vec::<u64>::new());
        add_head_block(&storage, 3, 1).await;
        add_head_block(&storage, 4, 1).await;
        assert_eq!(poll_log_blocks(&context, &id).await, vec![3, 4]);
        add_head_block(&storage, 5, 1).await;
        assert_eq!(poll_log_blocks(&context, &id).await, vec![5]);
    }

    /// A filter with a numeric `toBlock` reports the new logs up to that block, even when
    /// the chain has already moved past it by the time of the poll, and nothing after.
    #[tokio::test]
    async fn filter_changes_stop_at_the_filter_to_block() {
        let storage = Store::new("in-mem", EngineType::InMemory).unwrap();
        add_head_block(&storage, 0, 1).await;
        add_head_block(&storage, 1, 1).await;
        let context = default_context_with_storage(storage.clone()).await;
        let address = format!("{:#x}", ethrex_common::H160::from_low_u64_be(1));
        let id = map_http_requests(
            &rpc_call(
                "eth_newFilter",
                json!([{"address": address, "toBlock": "0x3"}]),
            ),
            context.clone(),
        )
        .await
        .unwrap();

        add_head_block(&storage, 2, 1).await;
        add_head_block(&storage, 3, 1).await;
        add_head_block(&storage, 4, 1).await;
        assert_eq!(poll_log_blocks(&context, &id).await, vec![2, 3]);
        add_head_block(&storage, 5, 1).await;
        assert_eq!(poll_log_blocks(&context, &id).await, Vec::<u64>::new());
    }

    /// A poll that fails leaves its blocks for the next poll instead of skipping them.
    #[tokio::test]
    async fn failed_filter_poll_does_not_advance_the_filter() {
        let storage = Store::new("in-mem", EngineType::InMemory).unwrap();
        add_head_block(&storage, 0, 1).await;
        add_head_block(&storage, 1, 1).await;
        let context = default_context_with_storage(storage.clone()).await;
        let address = format!("{:#x}", ethrex_common::H160::from_low_u64_be(1));
        let id = map_http_requests(
            &rpc_call("eth_newFilter", json!([{"address": address}])),
            context.clone(),
        )
        .await
        .unwrap();

        // Block 2 is the head before its receipts are stored, so reading its logs fails.
        let (block, receipts) = head_block(2, 1);
        let hash = add_head_without_receipts(&storage, block).await;
        let failed = map_http_requests(
            &rpc_call("eth_getFilterChanges", json!([id])),
            context.clone(),
        )
        .await;
        assert!(failed.is_err(), "the poll should fail: {failed:?}");

        storage.add_receipts(hash, receipts).await.unwrap();
        assert_eq!(poll_log_blocks(&context, &id).await, vec![2]);
        assert_eq!(poll_log_blocks(&context, &id).await, Vec::<u64>::new());
    }
}
