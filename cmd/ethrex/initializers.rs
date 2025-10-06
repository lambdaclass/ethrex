use crate::{
    cli::Options,
    utils::{
        display_chain_initialization, get_client_version, init_datadir, parse_socket_addr,
        read_jwtsecret_file, read_node_config_file,
    },
};
use ethrex_blockchain::{Blockchain, BlockchainOptions, BlockchainType};
use ethrex_common::{
    H256,
    types::{BlockHeader, Genesis},
};
use ethrex_config::networks::Network;

use ethrex_metrics::profiling::{FunctionProfilingLayer, initialize_block_processing_profile};
use ethrex_p2p::{
    kademlia::Kademlia,
    network::{P2PContext, peer_table},
    peer_handler::PeerHandler,
    rlpx::l2::l2_connection::P2PBasedContext,
    sync::insert_storages,
    sync_manager::SyncManager,
    types::{Node, NodeRecord},
    utils::{get_account_storages_snapshots_dir, public_key_from_signing_key},
};
use ethrex_storage::{EngineType, Store};
use local_ip_address::{local_ip, local_ipv6};
use rand::rngs::OsRng;
use secp256k1::SecretKey;
#[cfg(feature = "sync-test")]
use std::env;
use std::{
    collections::BTreeSet,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{
    EnvFilter, Layer, Registry, filter::Directive, fmt, layer::SubscriberExt, reload,
};

pub fn init_tracing(opts: &Options) -> reload::Handle<EnvFilter, Registry> {
    let log_filter = EnvFilter::builder()
        .with_default_directive(Directive::from(opts.log_level))
        .from_env_lossy();

    let (filter, filter_handle) = reload::Layer::new(log_filter);

    let fmt_layer = fmt::layer().with_filter(filter);
    let subscriber: Box<dyn tracing::Subscriber + Send + Sync> = if opts.metrics_enabled {
        let profiling_layer = FunctionProfilingLayer::default();
        Box::new(Registry::default().with(fmt_layer).with(profiling_layer))
    } else {
        Box::new(Registry::default().with(fmt_layer))
    };

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    filter_handle
}

pub fn init_metrics(opts: &Options, tracker: TaskTracker) {
    tracing::info!(
        "Starting metrics server on {}:{}",
        opts.metrics_addr,
        opts.metrics_port
    );
    let metrics_api = ethrex_metrics::api::start_prometheus_metrics_api(
        opts.metrics_addr.clone(),
        opts.metrics_port.clone(),
    );

    initialize_block_processing_profile();

    tracker.spawn(metrics_api);
}

/// Opens a new or pre-existing Store and loads the initial state provided by the network
pub async fn init_store(datadir: impl AsRef<Path>, genesis: Genesis) -> Store {
    let store = open_store(datadir.as_ref());
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to create genesis block");
    store
}

/// Initializes a pre-existing Store
pub async fn load_store(datadir: &Path) -> Store {
    let store = open_store(datadir);
    store
        .load_initial_state()
        .await
        .expect("Failed to load store");
    store
}

/// Opens a pre-existing Store or creates a new one
pub fn open_store(datadir: &Path) -> Store {
    if datadir.ends_with("memory") {
        Store::new(datadir, EngineType::InMemory).expect("Failed to create Store")
    } else {
        cfg_if::cfg_if! {
            if #[cfg(feature = "rocksdb")] {
                let engine_type = EngineType::RocksDB;
            } else if #[cfg(feature = "libmdbx")] {
                let engine_type = EngineType::Libmdbx;
            } else {
                error!("No database specified. The feature flag `rocksdb` or `libmdbx` should've been set while building.");
                panic!("Specify the desired database engine.");
            }
        };
        #[cfg(feature = "metrics")]
        ethrex_metrics::metrics_process::set_datadir_path(datadir.to_path_buf());
        Store::new(datadir, engine_type).expect("Failed to create Store")
    }
}

pub fn init_blockchain(store: Store, blockchain_opts: BlockchainOptions) -> Arc<Blockchain> {
    info!("Initiating blockchain with levm");
    Blockchain::new(store, blockchain_opts).into()
}

#[allow(clippy::too_many_arguments)]
pub async fn init_rpc_api(
    opts: &Options,
    peer_handler: PeerHandler,
    local_p2p_node: Node,
    local_node_record: NodeRecord,
    store: Store,
    blockchain: Arc<Blockchain>,
    cancel_token: CancellationToken,
    tracker: TaskTracker,
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
    gas_ceil: Option<u64>,
    extra_data: String,
) {
    init_datadir(&opts.datadir);
    // Create SyncManager
    let syncer = SyncManager::new(
        peer_handler.clone(),
        opts.syncmode.clone(),
        cancel_token,
        blockchain.clone(),
        store.clone(),
        opts.datadir.clone(),
    )
    .await;

    let rpc_api = ethrex_rpc::start_api(
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
        log_filter_handler,
        gas_ceil,
        extra_data,
    );

    tracker.spawn(rpc_api);
}

#[allow(clippy::too_many_arguments)]
pub async fn init_network(
    opts: &Options,
    network: &Network,
    datadir: &Path,
    local_p2p_node: Node,
    local_node_record: Arc<Mutex<NodeRecord>>,
    signer: SecretKey,
    peer_handler: PeerHandler,
    store: Store,
    tracker: TaskTracker,
    blockchain: Arc<Blockchain>,
    based_context: Option<P2PBasedContext>,
) {
    if opts.dev {
        error!("Binary wasn't built with The feature flag `dev` enabled.");
        panic!(
            "Build the binary with the `dev` feature in order to use the `--dev` cli's argument."
        );
    }

    let bootnodes = get_bootnodes(opts, network, datadir);

    let context = P2PContext::new(
        local_p2p_node,
        local_node_record,
        tracker.clone(),
        signer,
        peer_handler.peer_table.clone(),
        store,
        blockchain.clone(),
        get_client_version(),
        based_context,
    )
    .await
    .expect("P2P context could not be created");

    ethrex_p2p::start_network(context, bootnodes)
        .await
        .expect("Network starts");

    tracker.spawn(ethrex_p2p::periodically_show_peer_stats(
        blockchain,
        peer_handler.peer_table.peers.clone(),
    ));
}

#[cfg(feature = "dev")]
pub async fn init_dev_network(opts: &Options, store: &Store, tracker: TaskTracker) {
    info!("Running in DEV_MODE");

    let head_block_hash = {
        let current_block_number = store.get_latest_block_number().await.unwrap();
        store
            .get_canonical_block_hash(current_block_number)
            .await
            .unwrap()
            .unwrap()
    };

    let max_tries = 3;

    let url = format!(
        "http://{authrpc_socket_addr}",
        authrpc_socket_addr = get_authrpc_socket_addr(opts)
    );

    let block_producer_engine = ethrex_dev::block_producer::start_block_producer(
        url,
        read_jwtsecret_file(&opts.authrpc_jwtsecret),
        head_block_hash,
        max_tries,
        1000,
        ethrex_common::Address::default(),
    );
    tracker.spawn(block_producer_engine);
}

pub fn get_network(opts: &Options) -> Network {
    let default = if opts.dev {
        Network::LocalDevnet
    } else {
        Network::mainnet()
    };
    opts.network.clone().unwrap_or(default)
}

pub fn get_bootnodes(opts: &Options, network: &Network, datadir: &Path) -> Vec<Node> {
    let mut bootnodes: Vec<Node> = opts.bootnodes.clone();

    bootnodes.extend(network.get_bootnodes());

    debug!("Loading known peers from config");

    match read_node_config_file(datadir) {
        Ok(Some(ref mut config)) => bootnodes.append(&mut config.known_peers),
        Ok(None) => {} // No config file, nothing to do
        Err(e) => warn!("Could not read from peers file: {e}"),
    };

    if bootnodes.is_empty() {
        warn!("No bootnodes specified. This node will not be able to connect to the network.");
    }

    bootnodes
}

pub fn get_signer(datadir: &Path) -> SecretKey {
    // Get the signer from the default directory, create one if the key file is not present.
    let key_path = datadir.join("node.key");
    match fs::read(key_path.clone()) {
        Ok(content) => SecretKey::from_slice(&content).expect("Signing key could not be created."),
        Err(_) => {
            info!(
                "Key file not found, creating a new key and saving to {:?}",
                key_path
            );
            if let Some(parent) = key_path.parent() {
                fs::create_dir_all(parent).expect("Key file path could not be created.")
            }
            let signer = SecretKey::new(&mut OsRng);
            fs::write(key_path, signer.secret_bytes())
                .expect("Newly created signer could not be saved to disk.");
            signer
        }
    }
}

pub fn get_local_p2p_node(opts: &Options, signer: &SecretKey) -> Node {
    let udp_socket_addr = parse_socket_addr("::", &opts.discovery_port)
        .expect("Failed to parse discovery address and port");
    let tcp_socket_addr =
        parse_socket_addr("::", &opts.p2p_port).expect("Failed to parse addr and port");

    let p2p_node_ip = local_ip()
        .unwrap_or_else(|_| local_ipv6().expect("Neither ipv4 nor ipv6 local address found"));

    let local_public_key = public_key_from_signing_key(signer);

    let node = Node::new(
        p2p_node_ip,
        udp_socket_addr.port(),
        tcp_socket_addr.port(),
        local_public_key,
    );

    // TODO Find a proper place to show node information
    // https://github.com/lambdaclass/ethrex/issues/836
    let enode = node.enode_url();
    info!(enode = %enode, "Local node initialized");

    node
}

pub fn get_local_node_record(
    datadir: &Path,
    local_p2p_node: &Node,
    signer: &SecretKey,
) -> NodeRecord {
    match read_node_config_file(datadir) {
        Ok(Some(ref mut config)) => {
            NodeRecord::from_node(local_p2p_node, config.node_record.seq + 1, signer)
                .expect("Node record could not be created from local node")
        }
        _ => {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            NodeRecord::from_node(local_p2p_node, timestamp, signer)
                .expect("Node record could not be created from local node")
        }
    }
}

pub fn get_authrpc_socket_addr(opts: &Options) -> SocketAddr {
    parse_socket_addr(&opts.authrpc_addr, &opts.authrpc_port)
        .expect("Failed to parse authrpc address and port")
}

pub fn get_http_socket_addr(opts: &Options) -> SocketAddr {
    parse_socket_addr(&opts.http_addr, &opts.http_port)
        .expect("Failed to parse http address and port")
}

#[cfg(feature = "sync-test")]
async fn set_sync_block(store: &Store) {
    if let Ok(block_number) = env::var("SYNC_BLOCK_NUM") {
        let block_number = block_number
            .parse()
            .expect("Block number provided by environment is not numeric");
        let block_hash = store
            .get_canonical_block_hash(block_number)
            .await
            .expect("Could not get hash for block number provided by env variable")
            .expect("Could not get hash for block number provided by env variable");
        store
            .forkchoice_update(None, block_number, block_hash, None, None)
            .await
            .expect("Could not set sync block");
    }
}

pub async fn init_l1(
    opts: Options,
    log_filter_handler: Option<reload::Handle<EnvFilter, Registry>>,
) -> eyre::Result<(PathBuf, CancellationToken, Kademlia, Arc<Mutex<NodeRecord>>)> {
    let datadir = &opts.datadir;
    init_datadir(datadir);

    let network = get_network(&opts);

    let genesis = network.get_genesis()?;
    display_chain_initialization(&genesis);
    let store = init_store(datadir, genesis).await;

    let accounts_with_storage = BTreeSet::from_iter(
        [
            "00001df88acd7fefb0c45d32131782b5672d4b844ff686a7d2a027ef5dd1e48f",
            "000039e0fba30cca3d018633ae5327bfa2194626f6a551e7bb4b2c9d7ee8773e",
            "000042b14fef23a6d506da07804b570d1ec6d3af4d858ddd1133781c811de1c1",
            "0000e65fdfaa2681656a211a55bc6fdcfe918f34cc037407ba12874c16cd7da9",
            "0000f83782032b705359a8d3e92942d6b0582d2cbf765423e9e49c7ebea126e9",
            "00014aba332b5c65c3aa67c64bd9f7e013e33ccdbe336c0df225542df4cc3658",
            "00016f0657a459e7bcfbc645ce715cf70db7dbdaf7cbb869ec31f29d7cca5e9c",
            "0001791010668ddb98db9b6abcb13aad8a4ad07a6a56d06410fafe691e12044c",
            "00017f7910442eb26837312face39abd4e968051ee99432f9b71a3ac7ce7f657",
            "0001dd982b3c811cb40e9a6be2a9e8eaffc965caf1b97d38d62eface202a6f47",
            "000312b165de1a140de34ad48f5114be7a011f5f96da51dde1d926d1263065f6",
            "000342e99b931a065632cecc3357cfad23bbb785cba338497e5d7d15c93b0566",
            "000342ed85d7d23285bb1be7493b6ce9b7358e3aadb759b5322adee836e12e30",
            "00035dd5d5d2f814828ecae4a5a13e3e5a466abbc62554928dd07e2ff64d18d7",
            "00038d8253febbabedb4473b142b9cae3a2f20713a9fb4c9672c6b1cf6a9e94b",
            "0003c072d3e242350fa6b952bb293e0ce305df2c05f9285e221c72257f9092f7",
            "0003f82a39a455ca2248a3cab0b2dd7f43f70b2371c78ead74dcc123f392eab5",
            "000404bf9166e31e713ace18e5d9bd60d39b6dd02c9e931cfa476707fff8060c",
            "000466bf2a7602e7244290f872015807a61f8790bf057a099191aa468d1ad843",
            "0004ffcf0e4be522d07fa77856b566ff22d623d3981e1d1cc259e61bb3634f42",
            "0005ea12901faba3037eb78694b6882660044baaee97471737af570ffed0e565",
            "00060204b15f1832f8eac335b60cde8375701809a8110be78f16b790d96bfb41",
            "00066e7fe3f0c62dc98ac3a5dd4eb565fb3082dc33d9ed00006c58798e4da215",
            "0006755c3d0e96dcf9de12baf6e315df0c9ebb3038e58ba50b6781f33c93c4f7",
            "00068e2bb8f68e689e11dd44c7991a68ba920945183920cd4c216a65e2a0ce4b",
            "0006ab549ba8378123f21f3b3723ba7090bf0eaf7741dcf9f88d3a62377faeca",
            "0006b601b644941f83e4b32f8be4ed1bf49e173225b592394813fa2dce9c60eb",
            "0006fc6007d232e5228a68fdb165d569860c594f63ae0e92e61cc341d401fc53",
            "0007a879a8ae304da635e36252b3683c2ea18e43aae55bdafe0772d287dd87c2",
            "0007c5a2fe00e59a6107c1b2ff9c03578a55af26cc2b6b144be184440368a6be",
            "0007d22cc5966f2bc3ed2bb3ff76e637d8c3c620717eeb4598062140642f6e22",
            "00080ad3361676fe2dc54374d503859761b2ad4cd4c5cb587ae48f1d321a4bba",
            "00082ff83ca5d422853e0e695f731903698dcbaa839337394ed8f42433a2deb6",
            "0008424d0988ceca12de193b28bda176fd8cd5f19c0e80d71b11d4984432fcfd",
            "00087f36a10ecf9ecfadeba99f7ecaf22c5ab53d522367b4dd9369625f01b3dc",
            "0008a53712933dc7789005d630048621446ff5ae81227a6d08b6bcc1fead2009",
            "00092049bb26557840c389bb45d2fa9ccdc88cf70161b5ddf01d5308632bf98f",
            "0009461fa0298d53ed249a6fae8576e7334279436234fd2cb2ad690f2498ea93",
            "00095fd8e14ea68a445e7a3c45baa31f901e98dadae11763e332f0191876a1be",
            "0009674eec1d6ea882979de89a8f167ad3a35fe38dcb57be22639472748bfbeb",
            "00096f259a9fbe2e9192a4d7df90bcd160265a150ea25961d6c2c0f2ded556e2",
            "00097e40eecc14667b384fa22fe953fb646f87303f47c38ee1d5d9b3e8e1fa12",
            "00098aeca7957f58b8fac0da9056507067e265d3c7f7660fd481ae9550119f08",
            "00099879dabcf7fe9bf5f1f2763504b845f280c6b81af5eeab30ad7f4284c239",
            "0009b33abff856b7040b757a97ae751b95adec1ed069339b698e667650b3600f",
            "0009b7b5d334b02c07350b756273d84a31392ba07b8187cfbef93d6c1cad1c9d",
            "0009be02cbae63690ced0f8af2efd636e44998f443b68e23bc6f2669e1365989",
            "0009c6b13dea5eff217d39adc19b1f35c37c9d05640e621090a874e76ae8be86",
            "000a0ff02f83738fdd0c7e19adea2c15116748e5e42bfbfc6307461fe8f34553",
            "000a1d752c98f125dd8e37059a1ec86e77c98b4e6f185f8eeea1fe8cbcaf802d",
            "000a3781dcc14fc85ec1f0afcdbc39541ae56a181362780488675ea733ab71ff",
            "000a580672d4970d8b35d6dbb18245dab6a25fc9d27fe605d47daebce8b9bbb7",
            "000a5d875f6397c6ea0b993c6652cd078c526c8b32111b9707669c549bb964a0",
            "000a6ee6f9ca1b47b6ec7163c86833022eddcc1bf9c7ebd595602cca31eec3ed",
            "000a7d81d7b0e796b2768747e0d888c4bce22484d6dee6f723b5e6959271edc4",
            "000a9b64321925f1acd79083f490c8ee8ff1c798232bff0385299e16edcd9389",
        ]
        .iter()
        .map(|x| H256::from_str(x).unwrap()),
    );
    let _header = BlockHeader::default();
    let path = get_account_storages_snapshots_dir(datadir);
    insert_storages(store, accounts_with_storage, &path, datadir, &_header).await;
    std::process::exit(-1);

    #[cfg(feature = "sync-test")]
    set_sync_block(&store).await;

    let blockchain = init_blockchain(
        store.clone(),
        BlockchainOptions {
            max_mempool_size: opts.mempool_max_size,
            perf_logs_enabled: true,
            r#type: BlockchainType::L1,
        },
    );

    let signer = get_signer(datadir);

    let local_p2p_node = get_local_p2p_node(&opts, &signer);

    let local_node_record = Arc::new(Mutex::new(get_local_node_record(
        datadir,
        &local_p2p_node,
        &signer,
    )));

    let peer_handler = PeerHandler::new(peer_table());

    // TODO: Check every module starts properly.
    let tracker = TaskTracker::new();

    let cancel_token = tokio_util::sync::CancellationToken::new();

    init_rpc_api(
        &opts,
        peer_handler.clone(),
        local_p2p_node.clone(),
        local_node_record.lock().await.clone(),
        store.clone(),
        blockchain.clone(),
        cancel_token.clone(),
        tracker.clone(),
        log_filter_handler,
        // TODO (#4482): Make this configurable.
        None,
        opts.extra_data.clone(),
    )
    .await;

    if opts.metrics_enabled {
        init_metrics(&opts, tracker.clone());
    }

    if opts.dev {
        #[cfg(feature = "dev")]
        init_dev_network(&opts, &store, tracker.clone()).await;
    } else if opts.p2p_enabled {
        init_network(
            &opts,
            &network,
            datadir,
            local_p2p_node,
            local_node_record.clone(),
            signer,
            peer_handler.clone(),
            store.clone(),
            tracker.clone(),
            blockchain.clone(),
            None,
        )
        .await;
    } else {
        info!("P2P is disabled");
    }

    Ok((
        datadir.clone(),
        cancel_token,
        peer_handler.peer_table,
        local_node_record,
    ))
}
