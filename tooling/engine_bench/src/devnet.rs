//! Self-hosted per-fork devnets.
//!
//! Each fork gets a plain ethrex node (no `--dev`: the built-in dev producer
//! only speaks one getPayload version) on a patched copy of the local devnet
//! genesis, and the harness drives block production itself through the engine
//! API exactly like a CL would: fcU(attrs) → getPayload → newPayload → fcU.

use crate::cli::ForkArg;
use crate::setup;
use crate::transports::json_rpc;
use crate::workloads::ZERO_HASH;
use eyre::{Context, Result, eyre};
use reqwest::Client;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::{Child, Command};

/// The repo's local devnet genesis (Osaka-era); per-fork copies are derived
/// from it by trimming/adding fork activation times.
const GENESIS_TEMPLATE: &str = include_str!("../../../fixtures/genesis/l1.json");

fn fork_index(fork: ForkArg) -> usize {
    match fork {
        ForkArg::Paris => 0,
        ForkArg::Shanghai => 1,
        ForkArg::Cancun => 2,
        ForkArg::Prague => 3,
        ForkArg::Osaka => 4,
        ForkArg::Amsterdam => 5,
    }
}

/// Write a genesis activating exactly the forks up to `fork` at time 0.
pub fn write_genesis(fork: ForkArg, dir: &Path) -> Result<PathBuf> {
    let mut g: Value =
        serde_json::from_str(GENESIS_TEMPLATE).context("parsing genesis template")?;
    let cfg = g["config"]
        .as_object_mut()
        .ok_or_else(|| eyre!("genesis template has no config object"))?;

    let idx = fork_index(fork);
    const TIME_KEYS: [(&str, usize); 5] = [
        ("shanghaiTime", 1),
        ("cancunTime", 2),
        ("pragueTime", 3),
        ("osakaTime", 4),
        ("amsterdamTime", 5),
    ];
    for (key, key_idx) in TIME_KEYS {
        if key_idx > idx {
            cfg.remove(key);
        } else if !cfg.contains_key(key) {
            cfg.insert(key.to_string(), json!(0));
        }
    }
    // Keep only blob-schedule entries for active forks.
    let blob_fork_idx = |name: &str| match name {
        "cancun" => Some(2),
        "prague" => Some(3),
        "osaka" => Some(4),
        "amsterdam" => Some(5),
        _ => None,
    };
    if let Some(bs) = cfg.get_mut("blobSchedule").and_then(|v| v.as_object_mut()) {
        bs.retain(|k, _| blob_fork_idx(k).is_some_and(|i| i <= idx));
    }
    if cfg
        .get("blobSchedule")
        .and_then(|v| v.as_object())
        .is_some_and(|bs| bs.is_empty())
    {
        cfg.remove("blobSchedule");
    }

    let path = dir.join("genesis.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&g)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

pub struct DevNode {
    child: Child,
}

/// Start an ethrex node on the given genesis and wait for the authrpc port.
pub async fn spawn(
    ethrex_bin: &Path,
    genesis: &Path,
    datadir: &Path,
    jwt: &Path,
    port: u16,
) -> Result<DevNode> {
    // Never reuse a port something else (e.g. a real node) is listening on.
    if tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .is_ok()
    {
        return Err(eyre!(
            "port {port} is already in use — refusing to start a devnet on it \
             (pass --devnet-port to change)"
        ));
    }
    std::fs::create_dir_all(datadir)?;
    let log = std::fs::File::create(datadir.join("node.log"))?;
    let child = Command::new(ethrex_bin)
        .arg("--network")
        .arg(genesis)
        .arg("--datadir")
        .arg(datadir)
        .arg("--authrpc.port")
        .arg(port.to_string())
        .arg("--authrpc.jwtsecret")
        .arg(jwt)
        .arg("--http.port")
        .arg((port + 1).to_string())
        .arg("--p2p.disabled")
        // The default snap syncmode makes every fcU answer SYNCING on a fresh
        // chain, which blocks engine-API-driven block production.
        .arg("--syncmode")
        .arg("full")
        .stdout(log.try_clone()?)
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning {}", ethrex_bin.display()))?;

    for _ in 0..120 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return Ok(DevNode { child });
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(eyre!(
        "devnet did not open port {port} within 30s — see {}/node.log",
        datadir.display()
    ))
}

impl DevNode {
    pub async fn stop(mut self) -> Result<()> {
        self.child.kill().await.context("stopping devnet")
    }
}

/// JSON call that returns the parsed `result`, or errors on RPC error.
async fn call_ok(
    client: &Client,
    url: &str,
    token: &str,
    method: &str,
    params: impl serde::Serialize,
) -> Result<Value> {
    let resp = json_rpc::call(client, url, token, method, params).await?;
    let v = resp
        .json()
        .ok_or_else(|| eyre!("{method}: non-JSON response"))?;
    if v.get("error").is_some_and(|e| !e.is_null()) {
        return Err(eyre!("{method} error: {}", v["error"]));
    }
    Ok(v["result"].clone())
}

fn getpayload_method(fork: ForkArg) -> &'static str {
    match fork {
        ForkArg::Paris => "engine_getPayloadV1",
        ForkArg::Shanghai => "engine_getPayloadV2",
        ForkArg::Cancun => "engine_getPayloadV3",
        ForkArg::Prague => "engine_getPayloadV4",
        ForkArg::Osaka => "engine_getPayloadV5",
        ForkArg::Amsterdam => "engine_getPayloadV6",
    }
}

/// Drive block production until the chain reaches `target` blocks.
pub async fn produce_blocks(
    client: &Client,
    url: &str,
    secret: &[u8],
    fork: ForkArg,
    target: u64,
) -> Result<()> {
    loop {
        let token = crate::jwt::mint(secret)?;
        let height = call_ok(client, url, &token, "eth_blockNumber", json!([]))
            .await
            .ok()
            .and_then(|v| {
                v.as_str()
                    .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            })
            .unwrap_or(0);
        if height >= target {
            return Ok(());
        }

        let (head, _) = setup::latest_block(client, url, &token).await?;
        let pid = setup::start_payload_build(client, url, &token, fork, &head, head.timestamp + 1)
            .await?;
        // Empty-block builds are quick; give the async builder a moment.
        tokio::time::sleep(Duration::from_millis(80)).await;

        let result = call_ok(
            client,
            url,
            &token,
            getpayload_method(fork),
            (pid.as_str(),),
        )
        .await?;
        // V1 returns the bare payload; V2+ wrap it (and V4+ add requests).
        let (payload, requests) = if fork == ForkArg::Paris {
            (result, json!([]))
        } else {
            let requests = result
                .get("executionRequests")
                .cloned()
                .unwrap_or(json!([]));
            (result["executionPayload"].clone(), requests)
        };
        let block_hash = payload["blockHash"]
            .as_str()
            .ok_or_else(|| eyre!("built payload has no blockHash"))?
            .to_owned();

        let status = match fork {
            ForkArg::Paris => {
                call_ok(client, url, &token, "engine_newPayloadV1", (&payload,)).await?
            }
            ForkArg::Shanghai => {
                call_ok(client, url, &token, "engine_newPayloadV2", (&payload,)).await?
            }
            ForkArg::Cancun => {
                call_ok(
                    client,
                    url,
                    &token,
                    "engine_newPayloadV3",
                    (&payload, json!([]), ZERO_HASH),
                )
                .await?
            }
            ForkArg::Prague | ForkArg::Osaka => {
                call_ok(
                    client,
                    url,
                    &token,
                    "engine_newPayloadV4",
                    (&payload, json!([]), ZERO_HASH, &requests),
                )
                .await?
            }
            ForkArg::Amsterdam => {
                call_ok(
                    client,
                    url,
                    &token,
                    "engine_newPayloadV5",
                    (&payload, json!([]), ZERO_HASH, &requests),
                )
                .await?
            }
        };
        if status["status"] != "VALID" {
            return Err(eyre!("newPayload on built block: {status}"));
        }

        let fcu_state = json!({
            "headBlockHash": block_hash,
            "safeBlockHash": ZERO_HASH,
            "finalizedBlockHash": ZERO_HASH,
        });
        let (fcu_method, _) = setup::fcu_method_and_attrs(fork, 0, 0);
        call_ok(client, url, &token, fcu_method, (fcu_state, Value::Null)).await?;
    }
}
