// Backend bench measurements (`bench_backends_100`):
//
//   InMemory:  3 ms/fixture mean over 100 iters
//   RocksDB:   10 ms/fixture mean over 100 iters (tmpfs)
//   Host:      Linux debian 6.12.74+deb13+1-amd64 #1 SMP PREEMPT_DYNAMIC x86_64
//
// Targets: <5 ms InMemory, <50 ms RocksDB on tmpfs.

#[cfg(feature = "rocksdb")]
use std::path::PathBuf;

use ethrex_common::{H256, types::Genesis};
use ethrex_rpc::{
    RpcApiContext,
    rpc::{map_engine_requests, map_http_requests, rpc_response},
    utils::{RpcNamespace, RpcRequest, RpcRequestId},
};

use crate::engine_ctx::engine_only_context;
use ethrex_storage::{EngineType, Store};
use serde_json::Value;

/// Which storage backend to use when constructing a harness.
#[derive(Debug, Clone, Copy)]
pub enum Backend {
    /// Fully in-memory; fast, no disk I/O, no cleanup needed.
    InMemory,
    /// RocksDB on a temporary directory; cleaned up on `Drop` via `_tempdir`.
    /// Requires the `rocksdb` feature.
    #[cfg(feature = "rocksdb")]
    RocksDB,
}

/// A self-contained in-process harness for exercising the engine API against a
/// single test fixture. Each harness owns its own `Store` initialised from the
/// fixture's genesis; the shared `SyncManager` / `PeerHandler` scaffold is
/// allocated once per process (see `engine_only_context`).
pub struct EngineApiHarness {
    pub ctx: RpcApiContext,
    /// Keeps the RocksDB temp directory alive for the lifetime of the harness.
    /// `None` for `Backend::InMemory`.
    pub _tempdir: Option<tempfile::TempDir>,
}

/// Returns `/dev/shm` if it exists and is writable (tmpfs, good for RocksDB
/// benchmarks), otherwise falls back to `std::env::temp_dir()`.
#[cfg(feature = "rocksdb")]
fn prefer_tmpfs_dir() -> PathBuf {
    let shm = PathBuf::from("/dev/shm");
    if shm.is_dir()
        && std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(shm.join(".hive_write_probe"))
            .map(|_| std::fs::remove_file(shm.join(".hive_write_probe")).is_ok())
            .unwrap_or(false)
    {
        shm
    } else {
        std::env::temp_dir()
    }
}

impl EngineApiHarness {
    /// Build a harness from a typed `Genesis`. Hot path: callers should use this
    /// when they already hold a parsed `Genesis` to skip a round-trip through JSON.
    pub async fn from_genesis(genesis: Genesis, backend: Backend) -> anyhow::Result<Self> {
        let (store, tempdir) = match backend {
            Backend::InMemory => {
                let store = Store::new("", EngineType::InMemory)?;
                (store, None)
            }
            #[cfg(feature = "rocksdb")]
            Backend::RocksDB => {
                let dir = tempfile::TempDir::new_in(prefer_tmpfs_dir())?;
                let store = Store::new(dir.path(), EngineType::RocksDB)?;
                (store, Some(dir))
            }
        };

        let mut store = store;
        store.add_initial_state(genesis).await?;
        let ctx = engine_only_context(store).await;
        Ok(Self {
            ctx,
            _tempdir: tempdir,
        })
    }

    /// Convenience for callers that hold a JSON-encoded genesis (mostly tests).
    pub async fn from_genesis_json(genesis_json: &str, backend: Backend) -> anyhow::Result<Self> {
        let genesis: Genesis = serde_json::from_str(genesis_json)?;
        Self::from_genesis(genesis, backend).await
    }

    /// Dispatch a pre-built `RpcRequest` directly. Skips the JSON envelope
    /// round-trip — the per-method `serde_json::from_value::<T>(params)` inside
    /// each handler still exercises the serde path that matters for coverage.
    async fn dispatch(&self, req: RpcRequest) -> anyhow::Result<Value> {
        let res = match req.namespace() {
            Ok(RpcNamespace::Engine) => map_engine_requests(&req, self.ctx.clone()).await,
            Ok(_) => map_http_requests(&req, self.ctx.clone()).await,
            Err(e) => Err(e),
        };
        Ok(rpc_response(req.id.clone(), res)?)
    }

    /// Round-trip a JSON-RPC request body through the in-process dispatcher.
    /// Exercises the full envelope-parse path; used by external callers / tests.
    pub async fn call_raw(&self, body: &str) -> anyhow::Result<Value> {
        let req: RpcRequest = serde_json::from_str(body)?;
        self.dispatch(req).await
    }

    /// Build an RpcRequest and dispatch it directly (no envelope round-trip).
    async fn call(&self, method: &str, params: Vec<Value>) -> anyhow::Result<Value> {
        let req = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params: Some(params),
        };
        self.dispatch(req).await
    }

    /// Call `engine_forkchoiceUpdatedVx` with `head` as head, safe, and finalized hash.
    /// `version` must be 1–4; passes no payload attributes.
    pub async fn fcu(&self, version: u8, head: H256) -> anyhow::Result<Value> {
        let fcs = serde_json::json!({
            "headBlockHash": format!("{head:#x}"),
            "safeBlockHash": format!("{head:#x}"),
            "finalizedBlockHash": format!("{head:#x}"),
        });
        self.call(&format!("engine_forkchoiceUpdatedV{version}"), vec![fcs])
            .await
    }

    /// Call `engine_newPayloadVx`. `params` is the EEST fixture's pre-built params array.
    pub async fn new_payload(&self, version: u8, params: &[Value]) -> anyhow::Result<Value> {
        self.call(&format!("engine_newPayloadV{version}"), params.to_vec())
            .await
    }

    /// Call `engine_newPayloadWithWitnessV5` (same params as `engine_newPayloadV5`;
    /// the response additionally carries the geth-shaped `witness` blob).
    pub async fn new_payload_with_witness(&self, params: &[Value]) -> anyhow::Result<Value> {
        self.call("engine_newPayloadWithWitnessV5", params.to_vec())
            .await
    }

    /// Call `eth_getBlockByNumber("0x0", false)` and return the raw JSON response.
    pub async fn get_block_by_number_zero(&self) -> anyhow::Result<Value> {
        self.call(
            "eth_getBlockByNumber",
            vec![Value::String("0x0".to_string()), Value::Bool(false)],
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENESIS: &str = include_str!("../../../../fixtures/genesis/l1.json");

    /// Verify harness construction completes in reasonable time and the
    /// resulting context has a live syncer.
    #[tokio::test]
    async fn harness_builds_from_genesis() {
        let h = EngineApiHarness::from_genesis_json(GENESIS, Backend::InMemory)
            .await
            .expect("harness construction failed");
        assert!(h.ctx.syncer.is_some(), "syncer must be Some");
        assert!(h.ctx.peer_handler.is_none(), "peer_handler must be None");
    }

    /// Shared syncer: building two harnesses should reuse the same Arc.
    #[tokio::test]
    async fn shared_syncer_is_same_arc() {
        use std::sync::Arc;
        let h1 = EngineApiHarness::from_genesis_json(GENESIS, Backend::InMemory)
            .await
            .expect("first harness");
        let h2 = EngineApiHarness::from_genesis_json(GENESIS, Backend::InMemory)
            .await
            .expect("second harness");
        let p1 = Arc::as_ptr(h1.ctx.syncer.as_ref().unwrap());
        let p2 = Arc::as_ptr(h2.ctx.syncer.as_ref().unwrap());
        assert_eq!(p1, p2, "both harnesses must share the same SyncManager Arc");
    }

    /// Smoke test: RocksDB harness constructs and drops cleanly, tempdir is set.
    ///
    /// Requires the `rocksdb` feature: `cargo test -p ef_tests-engine --features rocksdb`.
    #[cfg(feature = "rocksdb")]
    #[tokio::test]
    async fn rocksdb_harness_builds_and_tempdir_is_some() {
        let h = EngineApiHarness::from_genesis_json(GENESIS, Backend::RocksDB)
            .await
            .expect("RocksDB harness construction failed");
        assert!(h._tempdir.is_some(), "_tempdir must be Some for RocksDB");
        drop(h); // TempDir RAII cleanup
    }

    /// Bench guard: mean per-fixture harness construction must stay below the
    /// stated targets for both backends over 100 iterations.
    ///
    /// Run with:
    ///   `cargo test -p ef_tests-engine --release --features rocksdb \
    ///       -- --include-ignored bench_backends_100`
    #[ignore = "timing guard; run with --release --include-ignored"]
    #[tokio::test]
    async fn bench_backends_100() {
        const ITERATIONS: u32 = 100;
        const INMEM_LIMIT_MS: u128 = 5;
        #[cfg(feature = "rocksdb")]
        const ROCKSDB_LIMIT_MS: u128 = 50;

        // InMemory
        let t0 = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            let _h = EngineApiHarness::from_genesis_json(GENESIS, Backend::InMemory)
                .await
                .expect("harness construction failed in bench");
        }
        let inmem_mean_ms = t0.elapsed().as_millis() / u128::from(ITERATIONS);

        // RocksDB (tmpfs when available)
        #[cfg(feature = "rocksdb")]
        let rocksdb_mean_ms = {
            let t1 = std::time::Instant::now();
            for _ in 0..ITERATIONS {
                let _h = EngineApiHarness::from_genesis_json(GENESIS, Backend::RocksDB)
                    .await
                    .expect("RocksDB harness construction failed in bench");
            }
            t1.elapsed().as_millis() / u128::from(ITERATIONS)
        };
        #[cfg(not(feature = "rocksdb"))]
        let rocksdb_mean_ms: u128 = 0;

        eprintln!(
            "bench_backends_100: InMemory={inmem_mean_ms} ms/iter  RocksDB={rocksdb_mean_ms} ms/iter  ratio={:.1}x",
            rocksdb_mean_ms as f64 / inmem_mean_ms.max(1) as f64,
        );

        assert!(
            inmem_mean_ms <= INMEM_LIMIT_MS,
            "InMemory mean ({inmem_mean_ms} ms) exceeded {INMEM_LIMIT_MS} ms limit"
        );
        #[cfg(feature = "rocksdb")]
        assert!(
            rocksdb_mean_ms <= ROCKSDB_LIMIT_MS,
            "RocksDB mean ({rocksdb_mean_ms} ms) exceeded {ROCKSDB_LIMIT_MS} ms limit"
        );
    }

    /// Verify `eth_getBlockByNumber("0x0", false)` succeeds and returns the genesis block.
    #[tokio::test]
    async fn eth_get_block_by_number_zero_returns_genesis() {
        let h = EngineApiHarness::from_genesis_json(GENESIS, Backend::InMemory)
            .await
            .expect("harness construction failed");

        let resp = h
            .get_block_by_number_zero()
            .await
            .expect("get_block_by_number_zero must succeed");

        // Must have no error field.
        assert!(
            resp.get("error").is_none(),
            "response must not have error: {resp}"
        );

        // result.hash must be a non-zero H256.
        let hash_str = resp["result"]["hash"]
            .as_str()
            .expect("result.hash must be a string");
        let hash: H256 = hash_str
            .parse()
            .expect("result.hash must be a valid H256 hex");
        assert_ne!(hash, H256::zero(), "genesis block hash must be non-zero");
    }
}
