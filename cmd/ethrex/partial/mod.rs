//! `ethrex partial-stateful-node` (alias `psn`): run ethrex as a partial stateful node.
//!
//! The node follows the chain through a sync committee light client, holds only the
//! accounts and storage its users read (every value proven against a verified header),
//! keeps them current with EIP-7928 block access lists, and optionally verifies EIP-8440
//! execution chain proofs for the chain it follows.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clap::{ArgAction, Args, Parser};
use ethrex_blockchain::{Blockchain, BlockchainOptions};
use ethrex_common::{Address, H256};
use ethrex_p2p::network::P2PContext;
use ethrex_p2p::peer_handler::PeerHandler;
use ethrex_p2p::peer_table::PeerTableServer;
use ethrex_p2p::rlpx::initiator::RLPxInitiator;
use ethrex_p2p::types::{LocalNode, SharedLocalNode};
use ethrex_partial::chain_proofs::engine::{
    DEFAULT_ENGINE_TIMEOUT, HttpProofEngine, MockProofEngine, ProofEngine,
};
use ethrex_partial::chain_proofs::tracker::ProofTracker;
use ethrex_partial::chain_proofs::{ChainProofs, ProofMode};
use ethrex_partial::fetch;
use ethrex_partial::follow::Follower;
use ethrex_partial::light_client::LightClient;
use ethrex_partial::light_client::beacon_api::BeaconApi;
use ethrex_partial::node::{Node, Shared};
use ethrex_partial::p2p_adapter::P2pPeers;
use ethrex_partial::peers::Peers;
use ethrex_partial::state::{Key, State};
use ethrex_partial::verify::TrustedHash;
use ethrex_partial::working::WorkingSet;
use ethrex_storage::{EngineType, Store};
use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{info, warn};
use url::Url;

use crate::cli::{Options, compute_effective_datadir};
use crate::initializers::{
    get_local_node_record, get_local_p2p_node, get_network, get_signer, init_network, init_tracing,
};
use crate::utils::get_client_version_string;

/// `ethrex partial-stateful-node`.
#[derive(Parser)]
pub struct PartialCommand {
    /// The node options shared with a full node: network, data directory, p2p, logging.
    #[clap(flatten)]
    pub opts: Options,
    /// Options specific to the partial node.
    #[clap(flatten)]
    pub partial: PartialOptions,
}

/// Options specific to the partial node.
#[derive(Args, Clone, Debug)]
pub struct PartialOptions {
    /// Beacon REST API the light client reads sync committee data from. It can stall the
    /// node but cannot make it accept an unsigned header.
    #[arg(
        long = "beacon.url",
        value_name = "URL",
        help_heading = "Partial node options"
    )]
    pub beacon_url: Url,

    /// Weak subjectivity checkpoint: a finalized beacon block root. Without it the beacon
    /// node's finalized block is trusted as the starting point.
    #[arg(
        long = "checkpoint",
        value_name = "BEACON_BLOCK_ROOT",
        help_heading = "Partial node options"
    )]
    pub checkpoint: Option<H256>,

    /// Accounts to keep proven at every head.
    #[arg(
        long = "watch",
        value_name = "ADDRESS",
        value_delimiter = ',',
        help_heading = "Partial node options"
    )]
    pub watch: Vec<Address>,

    /// Most keys kept proven, watched accounts included.
    #[arg(long = "working-set-cap", default_value_t = ethrex_partial::working::WORKING_SET_CAP, help_heading = "Partial node options")]
    pub working_set_cap: usize,

    /// How EIP-8440 execution chain proofs are used: off, observe or require.
    #[arg(
        long = "proof-mode",
        default_value = "off",
        help_heading = "Partial node options"
    )]
    pub proof_mode: ProofMode,

    /// Proof engine that verifies chain proofs (`POST /v1/execution_proof_verifications`).
    #[arg(
        long = "proof-engine.url",
        value_name = "URL",
        help_heading = "Partial node options"
    )]
    pub proof_engine_url: Option<Url>,

    /// Accept mock proofs instead of verifying them. Devnets and tests only: it proves
    /// nothing.
    #[arg(long = "proof-engine.mock", action = ArgAction::SetTrue, help_heading = "Partial node options")]
    pub proof_engine_mock: bool,

    /// Peers tried before the node gives up waiting for one that serves state, in seconds.
    #[arg(
        long = "peer-wait",
        default_value_t = 600,
        value_name = "SECONDS",
        help_heading = "Partial node options"
    )]
    pub peer_wait_secs: u64,

    /// Peers speaking eth to wait for before anchoring, so header requests have somewhere
    /// else to go when one peer cannot serve them.
    #[arg(
        long = "min-eth-peers",
        default_value_t = 3,
        value_name = "COUNT",
        help_heading = "Partial node options"
    )]
    pub min_eth_peers: usize,
}

impl PartialCommand {
    /// Runs the partial node until interrupted.
    pub async fn run(self) -> eyre::Result<()> {
        let (_filter, _guard) = init_tracing(&self.opts);
        let opts = self.opts;
        let partial = self.partial;

        let network = get_network(&opts);
        let genesis = network.get_genesis()?;
        let chain_config = genesis.config;
        let datadir = compute_effective_datadir(&opts.datadir, &network, false).join("partial");
        std::fs::create_dir_all(&datadir)?;
        info!(network = %network, datadir = %datadir.display(), "starting partial stateful node");

        // Headers only: the store backs the devp2p handshake and announcements. State is
        // never written to it.
        let mut store = Store::new("partial", EngineType::InMemory)?;
        store.add_initial_state(genesis).await?;
        let blockchain = Arc::new(Blockchain::new(
            store.clone(),
            BlockchainOptions {
                mempool_prewarm_enabled: false,
                ..Default::default()
            },
        ));

        let signer = get_signer(&datadir);
        let (local_node, network_config) = get_local_p2p_node(&opts, &signer);
        let record = get_local_node_record(&datadir, &local_node, &signer);
        let shared_local_node: SharedLocalNode = Arc::new(RwLock::new(LocalNode {
            node: local_node.clone(),
            record,
        }));
        let peer_table =
            PeerTableServer::spawn(local_node.node_id(), opts.target_peers, store.clone());
        let tracker = TaskTracker::new();
        let cancel = CancellationToken::new();
        let p2p_context = P2PContext::new(
            local_node,
            network_config,
            tracker.clone(),
            signer,
            peer_table.clone(),
            store.clone(),
            blockchain.clone(),
            get_client_version_string(),
            None,
            opts.tx_broadcasting_time_interval,
            opts.lookup_interval,
        )?;
        let initiator = RLPxInitiator::spawn(p2p_context.clone());
        let peer_handler = PeerHandler::new(peer_table.clone(), initiator);
        init_network(
            &opts,
            &network,
            &datadir,
            peer_handler.clone(),
            tracker.clone(),
            blockchain,
            p2p_context,
            shared_local_node,
        )
        .await;
        let peers: Arc<dyn Peers> = Arc::new(P2pPeers::new(peer_table, peer_handler, store));

        let beacon = BeaconApi::new(partial.beacon_url.clone(), Duration::from_secs(30))?;
        let light_client = LightClient::start(beacon.clone(), partial.checkpoint).await?;
        let checkpoint = light_client.checkpoint();
        let heads = light_client.heads()?;
        let (heads_rx, _light_client_task) = light_client.spawn(cancel.clone())?;

        wait_for_peers(
            peers.as_ref(),
            Duration::from_secs(partial.peer_wait_secs),
            partial.min_eth_peers,
        )
        .await?;
        // In `require` mode the node starts at the checkpoint's execution block, the trust
        // root, and only moves to payloads a verified chain proof covers.
        let start = match partial.proof_mode {
            ProofMode::Require => checkpoint.execution_block_hash,
            ProofMode::Off | ProofMode::Observe => heads.optimistic.execution_block_hash,
        };
        info!(hash = %start, "fetching the starting header from peers");
        let mut attempts = 0u64;
        let anchor = loop {
            match fetch::header(peers.as_ref(), TrustedHash::attested(start)).await {
                Ok(header) => break header,
                Err(err) => {
                    attempts += 1;
                    if attempts % 10 == 1 {
                        let census = peers.census().await;
                        warn!(
                            %err,
                            attempts,
                            eth_peers = census.eth_peers,
                            state_peers = census.state_peers,
                            "starting header not fetched yet; retrying"
                        );
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        };
        info!(number = anchor.number(), hash = %anchor.hash(), "anchored");

        let pins: Vec<Key> = partial.watch.iter().map(|a| Key::Account(*a)).collect();
        let keys_path = datadir.join("keys.json");
        let remembered = load_keys(&keys_path);
        let working = WorkingSet::new(partial.working_set_cap, pins, remembered);
        let shared = Shared::new(
            anchor,
            working,
            Arc::clone(&peers),
            chain_config,
            opts.rpc_gas_cap,
            partial.proof_mode,
        );
        let node = Node::new(Arc::clone(&shared), get_client_version_string());

        let proofs = match partial.proof_mode {
            ProofMode::Off => None,
            mode => {
                let engine: Arc<dyn ProofEngine> = if partial.proof_engine_mock {
                    warn!(
                        "mock proof engine enabled: chain proofs are accepted without verification"
                    );
                    Arc::new(MockProofEngine)
                } else if let Some(url) = partial.proof_engine_url.clone() {
                    Arc::new(HttpProofEngine::new(url, DEFAULT_ENGINE_TIMEOUT)?)
                } else {
                    eyre::bail!(
                        "--proof-mode {mode:?} needs --proof-engine.url or --proof-engine.mock"
                    );
                };
                Some(ChainProofs::new(
                    engine,
                    ProofTracker::new(checkpoint),
                    chain_config.chain_id,
                ))
            }
        };
        let beacon_for_proofs = (partial.proof_mode != ProofMode::Off).then(|| beacon.clone());
        let persist_path = keys_path.clone();
        let follower = Follower::new(shared, heads_rx, proofs, beacon_for_proofs, cancel.clone())
            .with_persist(Box::new(move |state: &State| {
                save_keys(&persist_path, state)
            }));
        tracker.spawn(follower.run());

        let addr: std::net::SocketAddr =
            format!("{}:{}", opts.http_addr, opts.http_port).parse()?;
        let rpc_cancel = cancel.clone();
        tracker.spawn(async move {
            if let Err(err) = ethrex_partial::rpc::serve(node, addr, rpc_cancel).await {
                warn!(%err, "JSON-RPC server stopped");
            }
        });

        tokio::signal::ctrl_c().await?;
        info!("shutting down");
        cancel.cancel();
        tracker.close();
        tokio::time::timeout(Duration::from_secs(10), tracker.wait())
            .await
            .ok();
        Ok(())
    }
}

async fn wait_for_peers(
    peers: &dyn Peers,
    limit: Duration,
    min_eth_peers: usize,
) -> eyre::Result<()> {
    let started = std::time::Instant::now();
    let mut logged = 0u64;
    loop {
        let census = peers.census().await;
        if census.state_peers > 0 && census.eth_peers >= min_eth_peers {
            info!(
                state_peers = census.state_peers,
                eth_peers = census.eth_peers,
                "peers connected"
            );
            return Ok(());
        }
        if started.elapsed() > limit {
            eyre::bail!(
                "no peer serving state connected within {}s",
                limit.as_secs()
            );
        }
        let elapsed = started.elapsed().as_secs();
        if elapsed / 10 > logged {
            logged = elapsed / 10;
            info!(
                eth_peers = census.eth_peers,
                state_peers = census.state_peers,
                min_eth_peers,
                "waiting for peers (one serving state, and enough speaking eth)"
            );
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[derive(Serialize, Deserialize)]
struct SavedKey {
    address: Address,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slot: Option<ethrex_common::U256>,
}

#[derive(Serialize, Deserialize, Default)]
struct SavedKeys {
    learned: Vec<SavedKey>,
}

fn load_keys(path: &Path) -> Vec<Key> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let saved: SavedKeys = match serde_json::from_str(&text) {
        Ok(saved) => saved,
        Err(err) => {
            warn!(%err, path = %path.display(), "ignoring unreadable saved keys");
            return Vec::new();
        }
    };
    saved
        .learned
        .into_iter()
        .map(|k| match k.slot {
            Some(slot) => Key::Slot(k.address, slot),
            None => Key::Account(k.address),
        })
        .collect()
}

fn save_keys(path: &PathBuf, state: &State) {
    let saved = SavedKeys {
        learned: state
            .working()
            .remembered()
            .into_iter()
            .map(|key| match key {
                Key::Account(address) => SavedKey {
                    address,
                    slot: None,
                },
                Key::Slot(address, slot) => SavedKey {
                    address,
                    slot: Some(slot),
                },
            })
            .collect(),
    };
    match serde_json::to_vec_pretty(&saved) {
        Ok(bytes) => {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes)
                .and_then(|()| std::fs::rename(&tmp, path))
                .is_err()
            {
                warn!(path = %path.display(), "could not save learned keys");
            }
        }
        Err(err) => warn!(%err, "could not encode learned keys"),
    }
}
