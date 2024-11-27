// The behaviour of the filtering endpoints is based on:
// - Manually testing the behaviour deploying contracts on the Sepolia test network.
// - Go-Ethereum, specifically: https://github.com/ethereum/go-ethereum/blob/368e16f39d6c7e5cce72a92ec289adbfbaed4854/eth/filters/filter.go
// - Ethereum's reference: https://ethereum.org/en/developers/docs/apis/json-rpc/#eth_newfilter
use ethrex_core::types::BlockNumber;
use ethrex_storage::Store;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tracing::error;

use crate::RpcHandler;
use crate::{
    types::block_identifier::{BlockIdentifier, BlockTag},
    utils::{parse_json_hex, RpcErr, RpcRequest},
};
use rand::prelude::*;
use serde_json::{json, Value};

use super::logs::{fetch_logs_with_filter, LogsFilter};

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

#[derive(Debug, Clone)]
pub struct PollableFilter {
    /// Last block number from when this
    /// filter was requested or created.
    /// i.e. if this filter is requested,
    /// the log will be applied from this
    /// block number up to the latest one.
    pub last_block_number: BlockNumber,
    pub filter_data: LogsFilter,
}

impl NewFilterRequest {
    pub fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, RpcErr> {
        let filter = LogsFilter::parse(params)?;
        Ok(NewFilterRequest {
            request_data: filter,
        })
    }

    pub fn handle(
        &self,
        storage: ethrex_storage::Store,
        filters: ActiveFilters,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        let from = self
            .request_data
            .from_block
            .resolve_block_number(&storage)?
            .ok_or(RpcErr::WrongParam("fromBlock".to_string()))?;
        let to = self
            .request_data
            .to_block
            .resolve_block_number(&storage)?
            .ok_or(RpcErr::WrongParam("toBlock".to_string()))?;

        if (from..=to).is_empty() {
            return Err(RpcErr::BadParams("Invalid block range".to_string()));
        }

        let Some(last_block_number) = storage.get_latest_block_number()? else {
            error!("Latest block number was requested but it does not exist");
            return Err(RpcErr::Internal("Failed to create filter".to_string()));
        };
        let id: u64 = random();
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
                    filter_data: self.request_data.clone(),
                },
            ),
        );
        let as_hex = json!(format!("0x{:x}", id));
        Ok(as_hex)
    }

    pub fn stateful_call(
        req: &RpcRequest,
        storage: Store,
        state: ActiveFilters,
    ) -> Result<Value, RpcErr> {
        let request = Self::parse(&req.params)?;
        request.handle(storage, state)
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
    pub fn handle(
        &self,
        storage: ethrex_storage::Store,
        filters: ActiveFilters,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        let Some(latest_block_num) = storage.get_latest_block_number()? else {
            error!("Latest block number was requested but it does not exist");
            return Err(RpcErr::Internal("Failed to create filter".to_string()));
        };
        let mut active_filters_guard = filters.lock().unwrap_or_else(|mut poisoned_guard| {
            error!("THREAD CRASHED WITH MUTEX TAKEN; SYSTEM MIGHT BE UNSTABLE");
            **poisoned_guard.get_mut() = HashMap::new();
            filters.clear_poison();
            poisoned_guard.into_inner()
        });
        if let Some((timestamp, filter)) = active_filters_guard.get_mut(&self.id) {
            // We'll only get changes for a filter that either has a block
            // range for upcoming blocks, or for the 'latest' tag.
            let valid_block_range = match filter.filter_data.to_block {
                BlockIdentifier::Tag(BlockTag::Latest) => true,
                BlockIdentifier::Number(block_num) if block_num >= latest_block_num => true,
                _ => false,
            };
            // This filter has a valid block range, so here's what we'll do:
            // - Update the filter's timestamp and block number from the last poll.
            // - Do the query to fetch logs in range last_block_number..=to_block for
            //   this filter.
            if valid_block_range {
                // Since the filter was polled, updated its timestamp, so
                // it does not expire.
                *timestamp = Instant::now();
                // Update this filter so the current query
                // starts from the last polled block.
                filter.filter_data.from_block = BlockIdentifier::Number(filter.last_block_number);
                filter.last_block_number = latest_block_num;
                let mut filter = filter.clone();
                filter.filter_data.to_block = BlockIdentifier::Number(latest_block_num);
                // Drop the lock early to process this filter's query
                // and not keep the lock more than we should.
                drop(active_filters_guard);
                let logs = fetch_logs_with_filter(&filter.filter_data, storage)?;
                serde_json::to_value(logs).map_err(|error| {
                    tracing::error!("Log filtering request failed with: {error}");
                    RpcErr::Internal("Failed to filter logs".to_string())
                })
            } else {
                serde_json::to_value(Vec::<u8>::new()).map_err(|error| {
                    tracing::error!("Log filtering request failed with: {error}");
                    RpcErr::Internal("Failed to filter logs".to_string())
                })
            }
        } else {
            Err(RpcErr::BadParams(
                "No matching filter for given id".to_string(),
            ))
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

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    use tokio::sync::Mutex as TokioMutex;

    use super::ActiveFilters;
    use crate::{
        eth::{
            filter::PollableFilter,
            logs::{AddressFilter, LogsFilter, TopicFilter},
        },
        map_http_requests,
        utils::test_utils::{self, start_test_api},
        RpcApiContext, FILTER_DURATION,
    };
    use crate::{
        types::block_identifier::BlockIdentifier,
        utils::{test_utils::example_p2p_node, RpcRequest},
    };
    use ethrex_core::types::Genesis;
    use ethrex_net::sync::SyncManager;
    use ethrex_storage::{EngineType, Store};

    use serde_json::{json, Value};
    use test_utils::TEST_GENESIS;

    #[test]
    fn filter_request_smoke_test_valid_params() {
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
        let id = run_new_filter_request_test(raw_json.clone(), filters.clone());
        let filters = filters.lock().unwrap();
        assert!(filters.len() == 1);
        let (_, filter) = filters.clone().get(&id).unwrap().clone();
        assert!(matches!(
            filter.filter_data.from_block,
            BlockIdentifier::Number(1)
        ));
        assert!(matches!(
            filter.filter_data.to_block,
            BlockIdentifier::Number(2)
        ));
        assert!(filter.filter_data.address_filters.is_none());
        assert!(matches!(
            &filter.filter_data.topics[..],
            [TopicFilter::Topic(_)]
        ));
    }

    #[test]
    fn filter_request_smoke_test_valid_null_topics_null_addr() {
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
        let id = run_new_filter_request_test(raw_json.clone(), filters.clone());
        let filters = filters.lock().unwrap();
        assert!(filters.len() == 1);
        let (_, filter) = filters.clone().get(&id).unwrap().clone();
        assert!(matches!(
            filter.filter_data.from_block,
            BlockIdentifier::Number(1)
        ));
        assert!(matches!(
            filter.filter_data.to_block,
            BlockIdentifier::Number(255)
        ));
        assert!(filter.filter_data.address_filters.is_none());
        assert!(matches!(&filter.filter_data.topics[..], []));
    }

    #[test]
    fn filter_request_smoke_test_valid_addr_topic_null() {
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
        let id = run_new_filter_request_test(raw_json.clone(), filters.clone());
        let filters = filters.lock().unwrap();
        assert!(filters.len() == 1);
        let (_, filter) = filters.clone().get(&id).unwrap().clone();
        assert!(matches!(
            filter.filter_data.from_block,
            BlockIdentifier::Number(1)
        ));
        assert!(matches!(
            filter.filter_data.to_block,
            BlockIdentifier::Number(255)
        ));
        assert!(matches!(
            filter.filter_data.address_filters.unwrap(),
            AddressFilter::Many(_)
        ));
        assert!(matches!(&filter.filter_data.topics[..], []));
    }

    #[test]
    #[should_panic]
    fn filter_request_smoke_test_invalid_block_range() {
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
        run_new_filter_request_test(raw_json.clone(), Default::default());
    }

    #[test]
    #[should_panic]
    fn filter_request_smoke_test_from_block_missing() {
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
        run_new_filter_request_test(raw_json.clone(), filters.clone());
    }

    fn run_new_filter_request_test(
        json_req: serde_json::Value,
        filters_pointer: ActiveFilters,
    ) -> u64 {
        let context = RpcApiContext {
            storage: Store::new("in-mem", EngineType::InMemory)
                .expect("Fatal: could not create in memory test db"),
            jwt_secret: Default::default(),
            local_p2p_node: example_p2p_node(),
            active_filters: filters_pointer.clone(),
            syncer: Arc::new(TokioMutex::new(SyncManager::dummy())),
        };
        let request: RpcRequest = serde_json::from_value(json_req).expect("Test json is incorrect");
        let genesis_config: Genesis =
            serde_json::from_str(TEST_GENESIS).expect("Fatal: non-valid genesis test config");

        context
            .storage
            .add_initial_state(genesis_config)
            .expect("Fatal: could not add test genesis in test");
        let response = map_http_requests(&request, context).unwrap().to_string();
        let trimmed_id = response.trim().trim_matches('"');
        assert!(trimmed_id.starts_with("0x"));
        let hex = trimmed_id.trim_start_matches("0x");
        let parsed = u64::from_str_radix(hex, 16);
        assert!(u64::from_str_radix(hex, 16).is_ok());
        parsed.unwrap()
    }

    #[test]
    fn install_filter_removed_correctly_test() {
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
                    filter_data: LogsFilter {
                        from_block: BlockIdentifier::Number(1),
                        to_block: BlockIdentifier::Number(2),
                        address_filters: None,
                        topics: vec![],
                    },
                },
            ),
        );
        let active_filters = Arc::new(Mutex::new(HashMap::from([filter])));
        let context = RpcApiContext {
            storage: Store::new("in-mem", EngineType::InMemory).unwrap(),
            local_p2p_node: example_p2p_node(),
            jwt_secret: Default::default(),
            active_filters: active_filters.clone(),
            syncer: Arc::new(TokioMutex::new(SyncManager::dummy())),
        };

        map_http_requests(&uninstall_filter_req, context).unwrap();

        assert!(
            active_filters.clone().lock().unwrap().len() == 0,
            "Expected filter map to be empty after request"
        );
    }

    #[test]
    fn removing_non_existing_filter_returns_false() {
        let active_filters = Arc::new(Mutex::new(HashMap::new()));

        let context = RpcApiContext {
            storage: Store::new("in-mem", EngineType::InMemory).unwrap(),
            local_p2p_node: example_p2p_node(),
            active_filters: active_filters.clone(),
            jwt_secret: Default::default(),
            syncer: Arc::new(TokioMutex::new(SyncManager::dummy())),
        };
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
        let res = map_http_requests(&uninstall_filter_req, context).unwrap();
        assert!(matches!(res, serde_json::Value::Bool(false)));
    }

    #[tokio::test]
    async fn background_job_removes_filter_smoke_test() {
        // Start a test server to start the cleanup
        // task in the background
        let server_handle = tokio::spawn(async move { start_test_api().await });

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
}
