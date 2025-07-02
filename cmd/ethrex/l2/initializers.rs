use std::fs::read_to_string;
use std::sync::Arc;

use ethrex_blockchain::Blockchain;
use ethrex_common::Address;
use ethrex_l2::based::sequencer_state::SequencerState;
use ethrex_p2p::kademlia::KademliaTable;
use ethrex_p2p::peer_handler::PeerHandler;
use ethrex_p2p::sync_manager::SyncManager;
use ethrex_p2p::types::{Node, NodeRecord};
use ethrex_storage::Store;
use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::warn;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tui_logger::{LevelFilter, TuiTracingSubscriberLayer};

use crate::cli::Options as L1Options;
use crate::initializers::{get_authrpc_socket_addr, get_http_socket_addr};
use crate::l2::L2Options;
use crate::utils::{get_client_version, read_jwtsecret_file};

#[allow(clippy::too_many_arguments)]
pub async fn init_rpc_api(
    opts: &L1Options,
    l2_opts: &L2Options,
    peer_table: Arc<Mutex<KademliaTable>>,
    local_p2p_node: Node,
    local_node_record: NodeRecord,
    store: Store,
    blockchain: Arc<Blockchain>,
    cancel_token: CancellationToken,
    tracker: TaskTracker,
    rollup_store: StoreRollup,
    initial_state: SequencerState,
) {
    let peer_handler = PeerHandler::new(peer_table);

    // Create SyncManager
    let syncer = SyncManager::new(
        peer_handler.clone(),
        opts.syncmode.clone(),
        cancel_token,
        blockchain.clone(),
        store.clone(),
    )
    .await;

    let rpc_api = ethrex_l2_rpc::start_api(
        get_http_socket_addr(opts),
        get_authrpc_socket_addr(opts),
        store,
        blockchain,
        read_jwtsecret_file(&opts.authrpc_jwtsecret),
        local_p2p_node,
        local_node_record,
        syncer,
        peer_handler,
        get_client_version(),
        get_valid_delegation_addresses(l2_opts),
        l2_opts.sponsor_private_key,
        rollup_store,
        initial_state,
    );

    tracker.spawn(rpc_api);
}

pub fn get_valid_delegation_addresses(l2_opts: &L2Options) -> Vec<Address> {
    let Some(ref path) = l2_opts.sponsorable_addresses_file_path else {
        warn!("No valid addresses provided, ethrex_SendTransaction will always fail");
        return Vec::new();
    };
    let addresses: Vec<Address> = read_to_string(path)
        .unwrap_or_else(|_| panic!("Failed to load file {path}"))
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.to_string().parse::<Address>())
        .filter_map(Result::ok)
        .collect();
    if addresses.is_empty() {
        warn!("No valid addresses provided, ethrex_SendTransaction will always fail");
    }
    addresses
}

pub async fn init_rollup_store(data_dir: &str) -> StoreRollup {
    cfg_if::cfg_if! {
        if #[cfg(feature = "rollup_storage_sql")] {
            let engine_type = EngineTypeRollup::SQL;
        } else if #[cfg(feature = "rollup_storage_redb")] {
            let engine_type = EngineTypeRollup::RedB;
        } else if #[cfg(feature = "rollup_storage_libmdbx")] {
            let engine_type = EngineTypeRollup::Libmdbx;
        } else {
            let engine_type = EngineTypeRollup::InMemory;
        }
    }
    let rollup_store =
        StoreRollup::new(data_dir, engine_type).expect("Failed to create StoreRollup");
    rollup_store
        .init()
        .await
        .expect("Failed to init rollup store");
    rollup_store
}

pub fn init_tracing() {
    let level_filter = EnvFilter::builder()
        .parse_lossy("debug,tower_http::trace=debug,reqwest_tracing=off,hyper=off,libsql=off,ethrex::initializers=off,ethrex::l2::initializers=off,ethrex::l2::command=off");
    let subscriber = tracing_subscriber::registry()
        .with(TuiTracingSubscriberLayer)
        .with(level_filter);
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
    tui_logger::init_logger(LevelFilter::max()).expect("Failed to initialize tui_logger");
}
