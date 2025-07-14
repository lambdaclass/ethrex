use std::{
    net::{IpAddr, Ipv4Addr},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use ethrex_blockchain::{Blockchain, BlockchainType};
use ethrex_common::H512;
use ethrex_p2p_2::{
    discv4::{server::DiscoveryServer, side_car::DiscoverySideCar},
    kademlia::Kademlia,
    metrics::METRICS,
    monitor::{app::Monitor, init_terminal, restore_terminal},
    network::P2PContext,
    rlpx::initiator::RLPxInitiator,
    types::{Node, NodeRecord},
};
use ethrex_storage::Store;
use ethrex_vm::EvmEngine;
use k256::{PublicKey, ecdsa::SigningKey, elliptic_curve::sec1::ToEncodedPoint};
use rand::rngs::OsRng;
use tokio::{net::UdpSocket, sync::Mutex};
use tokio_util::task::TaskTracker;
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, FmtSubscriber, filter::Directive, layer::SubscriberExt};
use tui_logger::{LevelFilter, TuiTracingSubscriberLayer};

pub const HOLESKY_GENESIS_CONTENTS: &str =
    include_str!("../../../../cmd/ethrex/networks/holesky/genesis.json");
pub const HOLESKY_BOOTNODES_ENODES: [&str; 4] = [
    "enode://ac906289e4b7f12df423d654c5a962b6ebe5b3a74cc9e06292a85221f9a64a6f1cfdd6b714ed6dacef51578f92b34c60ee91e9ede9c7f8fadc4d347326d95e2b@146.190.13.128:30303",
    "enode://a3435a0155a3e837c02f5e7f5662a2f1fbc25b48e4dc232016e1c51b544cb5b4510ef633ea3278c0e970fa8ad8141e2d4d0f9f95456c537ff05fdf9b31c15072@178.128.136.233:30303",
    "enode://7fa09f1e8bb179ab5e73f45d3a7169a946e7b3de5ef5cea3a0d4546677e4435ee38baea4dd10b3ddfdc1f1c5e869052932af8b8aeb6f9738598ec4590d0b11a6@65.109.94.124:30303",
    "enode://3524632a412f42dee4b9cc899b946912359bb20103d7596bddb9c8009e7683b7bff39ea20040b7ab64d23105d4eac932d86b930a605e632357504df800dba100@172.174.35.249:30303",
];

pub const HOODI_GENESIS_CONTENTS: &str =
    include_str!("../../../../cmd/ethrex/networks/hoodi/genesis.json");
pub const HOODI_BOOTNODES_ENODES: [&str; 16] = [
    "enode://ecbdd0a859c44067287963ed739e047140c52329f2876892f375e91e99e285c32f5f56943c3df605fe35b7c3113a366730994d0ba465883173f3aedbe028579b@3.38.213.47:64011",
    "enode://49df6667cac407e5d131d6c600b45b0afeccf89b8af0b727f547ec6f9cffca537c851fa8b62dbb35d72fe3f93b678344e02c02c2a6b11c313381c39e12fc08f7@178.162.204.214:60598",
    "enode://51fdefc893f45752d141a179f095ef52342fa801c9d9808716bd6d78123381d7a397d1ed30f02baa03b73d0f0c288e520622ca8c2c83f2cd78314da763343044@47.32.96.208:35260",
    "enode://9ed532cedafda9b8395e9ee5620e54b13207bb72edfe6708c0758e29a1ab9b4ae7836f4aab6fb3978978f46030f3f697d5faf3b126276ca625f1e48cdb4c55ee@141.95.98.128:15761",
    "enode://871c5a892c0fb40bfc1b6c696559d6b1bcbd02f60921b400b20e2629cf38d7d3aaafece97db3525bce37f2a9d7a31055e3e006eac5580e90a58f28adea384fc9@141.94.143.182:30303",
    "enode://e70dc434ae34f6df8c653f0a3b9449ec9769478bf0ac351b1845db772a96a529c776c27d640042756754b4d2041229ad50c2c12e36cb100f6a8a58d1e886dcfc@23.111.184.82:30303",
    "enode://30eab529d65a86a66d905c878ce9bdb55d1904a484d410310e6eeca8a2a2225461f86e58732e5e499061c9ddb80d1ec00e96ad6a5e596d42445d5f3752fb96ba@113.43.234.98:61487",
    "enode://f4736654a0f9fb9e45db6d87eef524392cb576d4971e9f14a02dafd8191557dba648088f14eab0503bbc33559847bc027cd7ad0b4c3f1efa6245ab724110c623@193.35.50.208:47894",
    "enode://2112dd3839dd752813d4df7f40936f06829fc54c0e051a93967c26e5f5d27d99d886b57b4ffcc3c475e930ec9e79c56ef1dbb7d86ca5ee83a9d2ccf36e5c240c@134.209.138.84:30303",
    "enode://55f925e283d160b156ad7564476a6595c9d9d6b307f3ce73fa42dc5d81dae264a6aca7115898b86e7640fdb515887f2f0afe6953ed761ff3681b4668ac69d2d9@73.231.50.199:30303?discport=29415",
    "enode://0f06ba68fbcb4c63205582c49e4ea318b2805986f1f2a796649fbb393c80e24f73748d4cb8a8a7769f3807a89193d965e5610ca56fccbc824c2b1981650f12d6@162.55.232.246:30303",
    "enode://60203fcb3524e07c5df60a14ae1c9c5b24023ea5d47463dfae051d2c9f3219f309657537576090ca0ae641f73d419f53d8e8000d7a464319d4784acd7d2abc41@209.38.124.160:30303",
    "enode://0533eb1233c9214039822c7bf17496c29ebcc477b89a85805a708d0cc270ad9e3fd1d184004e943a53160251cfda5aa4bf2f068c43120cb048775d919a0968a5@65.109.125.217:53096",
    "enode://f2ca294acfbd6638a35dd219ec7e384cc8133cdd41a8a9e65a56a57a898fea8b79d14814f3cc184f17374b9b14052310087c16dfdb3525bf19fd187e326a0a48@91.99.30.108:30303",
    "enode://6a88ab75521c6c28bcd1ff6e3fe96bc06c1c21deb10ee2c4d989570bca398e9e2526439c076bb226f86b88a3eb305cf66b2447130357d3590f77f7f75a0aad07@57.128.20.31:30303",
    "enode://fd2760f45525b1e3a6d7d87e457f7f158540716da3755dcc4be6664d34ab50dea3a552414a60eb8787349faf45d68eb8081116aa42a6746fde616e5b6e934d82@202.137.165.13:47500",
];

pub const MAINNET_GENESIS_CONTENTS: &str =
    include_str!("../../../../cmd/ethrex/networks/mainnet/genesis.json");
pub const MAINNET_BOOTNODES_ENODES: [&str; 16] = [
    // Ethereum Foundation Go Bootnodes
    "enode://4aeb4ab6c14b23e2c4cfdce879c04b0748a20d8e9b59e25ded2a08143e265c6c25936e74cbc8e641e3312ca288673d91f2f93f8e277de3cfa444ecdaaf982052@157.90.35.166:30303",
    "enode://2b252ab6a1d0f971d9722cb839a42cb81db019ba44c08754628ab4a823487071b5695317c8ccd085219c3a03af063495b2f1da8d18218da2d6a82981b45e6ffc@65.108.70.101:30303",
    "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303", // bootnode-aws-ap-southeast-1-001
    "enode://22a8232c3abc76a16ae9d6c3b164f98775fe226f0917b0ca871128a74a8e9630b458460865bab457221f1d448dd9791d24c4e5d88786180ac185df813a68d4de@3.209.45.79:30303", // bootnode-aws-us-east-1-001
    "enode://ca6de62fce278f96aea6ec5a2daadb877e51651247cb96ee310a318def462913b653963c155a0ef6c7d50048bba6e6cea881130857413d9f50a621546b590758@34.255.23.113:30303", // bootnode-aws-eu-west-1-001
    "enode://279944d8dcd428dffaa7436f25ca0ca43ae19e7bcf94a8fb7d1641651f92d121e972ac2e8f381414b80cc8e5555811c2ec6e1a99bb009b3f53c4c69923e11bd8@35.158.244.151:30303", // bootnode-aws-eu-central-1-001
    "enode://8499da03c47d637b20eee24eec3c356c9a2e6148d6fe25ca195c7949ab8ec2c03e3556126b0d7ed644675e78c4318b08691b7b57de10e5f0d40d05b09238fa0a@52.187.207.27:30303", // bootnode-azure-australiaeast-001
    "enode://103858bdb88756c71f15e9b5e09b56dc1be52f0a5021d46301dbbfb7e130029cc9d0d6f73f693bc29b665770fff7da4d34f3c6379fe12721b5d7a0bcb5ca1fc1@191.234.162.198:30303", // bootnode-azure-brazilsouth-001
    "enode://715171f50508aba88aecd1250af392a45a330af91d7b90701c436b618c86aaa1589c9184561907bebbb56439b8f8787bc01f49a7c77276c58c1b09822d75e8e8@52.231.165.108:30303", // bootnode-azure-koreasouth-001
    "enode://5d6d7cd20d6da4bb83a1d28cadb5d409b64edf314c0335df658c1a54e32c7c4a7ab7823d57c39b6a757556e68ff1df17c748b698544a55cb488b52479a92b60f@104.42.217.25:30303", // bootnode-azure-westus-001
    // Ethereum Foundation Go Bootnodes (legacy)
    "enode://a979fb575495b8d6db44f750317d0f4622bf4c2aa3365d6af7c284339968eef29b69ad0dce72a4d8db5ebb4968de0e3bec910127f134779fbcb0cb6d3331163c@52.16.188.185:30303", // IE
    "enode://3f1d12044546b76342d59d4a05532c14b85aa669704bfe1f864fe079415aa2c02d743e03218e57a33fb94523adb54032871a6c51b2cc5514cb7c7e35b3ed0a99@13.93.211.84:30303", // US-WEST
    "enode://78de8a0916848093c73790ead81d1928bec737d565119932b98c6b100d944b7a95e94f847f689fc723399d2e31129d182f7ef3863f2b4c820abbf3ab2722344d@191.235.84.50:30303", // BR
    "enode://158f8aab45f6d19c6cbf4a089c2670541a8da11978a2f90dbf6a502a4a3bab80d288afdbeb7ec0ef6d92de563767f3b1ea9e8e334ca711e9f8e2df5a0385e8e6@13.75.154.138:30303", // AU
    "enode://1118980bf48b0a3640bdba04e0fe78b1add18e1cd99bf22d53daac1fd9972ad650df52176e7c7d89d1114cfef2bc23a2959aa54998a46afcf7d91809f0855082@52.74.57.123:30303", // SG
    // Ethereum Foundation C++ Bootnodes
    "enode://979b7fa28feeb35a4741660a16076f1943202cb72b6af70d327f053e248bab9ba81760f39d0701ef1d8f89cc1fbd2cacba0710a12cd5314d5e0c9021aa3637f9@5.1.83.226:30303", // DE
];

#[tokio::main]
async fn main() {
    init_tracing();

    let signer = SigningKey::random(&mut OsRng);

    let local_node = Node::new(
        IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
        30303,
        30303,
        public_key_from_signing_key(&signer),
    );

    let kademlia = Kademlia::new();

    let udp_socket = Arc::new(
        UdpSocket::bind(local_node.udp_addr())
            .await
            .expect("Failed to bind udp socket"),
    );

    let _ = DiscoveryServer::spawn(
        local_node.clone(),
        signer.clone(),
        udp_socket.clone(),
        kademlia.clone(),
        bootnodes(&MAINNET_BOOTNODES_ENODES),
    )
    .await
    .inspect_err(|e| {
        error!("Failed to start discovery server: {e}");
    });

    let _ = DiscoverySideCar::spawn(
        local_node.clone(),
        signer.clone(),
        udp_socket,
        kademlia.clone(),
    )
    .await
    .inspect_err(|e| {
        error!("Failed to start discovery side car: {e}");
    });

    let local_node_record =
        NodeRecord::from_node(&local_node, 1, &signer).expect("Failed to create local node record");

    let store =
        Store::new("./db", ethrex_storage::EngineType::InMemory).expect("Failed to create store");

    let genesis =
        serde_json::from_str(MAINNET_GENESIS_CONTENTS).expect("Failed to parse genesis JSON");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to create genesis block");

    let blockchain = Blockchain::new(EvmEngine::LEVM, store.clone(), BlockchainType::L1).into();

    let context = P2PContext::new(
        local_node.clone(),
        Arc::new(Mutex::new(local_node_record)),
        TaskTracker::new(),
        signer.clone(),
        kademlia.clone(),
        store,
        blockchain,
        "0.0.1".to_owned(),
    );

    let _ = RLPxInitiator::spawn(context, local_node, signer, kademlia.clone())
        .await
        .inspect_err(|e| {
            error!("Failed to start RLPx Initiator: {e}");
        });

    let kademlia_counter_handle = tokio::spawn(async move {
        let start = std::time::Instant::now();
        loop {
            info!(
                r#"
elapsed: {}
{} contacts ({} contacts/s)
{} peers ({} new peers/s)
{} connection attempts ({} new connection attempts/s)
{} failed connections
failures: {:#?}"#,
                format_duration(start.elapsed()),
                METRICS.contacts.get(),
                METRICS.new_contacts_rate.get().floor(),
                METRICS.rlpx_conn_establishments.get(),
                METRICS.rlpx_conn_establishments_rate.get().floor(),
                METRICS.rlpx_conn_attempts.get(),
                METRICS.rlpx_conn_attempts_rate.get().floor(),
                METRICS.rlpx_conn_failures.get(),
                METRICS.rlpx_conn_failures_reasons_counts.lock().await,
            );
            // info!(
            //     contacts = kademlia_clone.table.lock().await.len(),
            //     number_of_peers = number_of_peers,
            //     number_of_tried_peers = number_of_tried_peers,
            //     elapsed = format_duration(elapsed),
            //     new_contacts_rate = %format!("{} contacts/s", METRICS.new_contacts_rate.get().floor()),
            //     connection_attempts_rate = %format!("{} attempts/s", METRICS.attempted_rlpx_conn_rate.get().floor()),
            //     connection_establishments_rate = %format!("{} establishments/s", METRICS.established_rlpx_conn_rate.get().floor()),
            //     failed_connections = METRICS.failed_rlpx_conn.get(),
            // );
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    // let mut terminal = init_terminal().expect("Failed to initialize terminal");

    // let mut monitor = Monitor::new("Ethrex P2P");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("Received Ctrl+C, shutting down...");
            // restore_terminal(&mut terminal).expect("Failed to restore terminal");
            kademlia_counter_handle.abort();
            store_peers_in_file(kademlia).await;
        }
        // _ = monitor.start(&mut terminal) => {
        //     println!("Monitor has exited, shutting down...");
        //     restore_terminal(&mut terminal).expect("Failed to restore terminal");
        //     kademlia_counter_handle.abort();
        // }
    }
}

pub fn init_tracing() {
    let log_filter = EnvFilter::builder().from_env_lossy();
    // .add_directive(Directive::from(opts.log_level));
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(log_filter)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
}

// pub fn init_tracing() {
//     let level_filter = EnvFilter::builder().parse_lossy("debug");
//     let subscriber = tracing_subscriber::registry()
//         .with(TuiTracingSubscriberLayer)
//         .with(level_filter);
//     tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
//     tui_logger::init_logger(LevelFilter::max()).expect("Failed to initialize tui_logger");
// }

pub fn public_key_from_signing_key(signer: &SigningKey) -> H512 {
    let public_key = PublicKey::from(signer.verifying_key());
    let encoded = public_key.to_encoded_point(false);
    H512::from_slice(&encoded.as_bytes()[1..])
}

pub fn bootnodes(bootnodes_enodes: &[&str]) -> Vec<Node> {
    bootnodes_enodes
        .iter()
        .map(|&s| Node::from_str(s).expect("Failed to parse bootnode enode"))
        .collect()
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

async fn _read_peers_from_file() -> Vec<Node> {
    tokio::fs::read("peers.json")
        .await
        .map(|data| serde_json::from_slice::<Vec<Node>>(&data).unwrap_or_default())
        .unwrap_or_default()
}

async fn store_peers_in_file(kademlia: Kademlia) {
    let peers_node_ids = kademlia
        .peers
        .lock()
        .await
        .iter()
        .cloned()
        .collect::<Vec<_>>();

    let current_peers = kademlia
        .table
        .lock()
        .await
        .iter()
        .filter_map(|(node_id, node)| peers_node_ids.contains(node_id).then_some(node))
        .cloned()
        .collect::<Vec<_>>();

    let total_known_peers = tokio::fs::read("peers.json")
        .await
        .ok()
        .and_then(|data| serde_json::from_slice::<Vec<Node>>(&data).ok())
        .unwrap_or_default();

    let new_peers = current_peers
        .iter()
        .filter(|contact| {
            !total_known_peers
                .iter()
                .any(|already_known_peer| already_known_peer.node_id() == contact.node.node_id())
        })
        .map(|contact| &contact.node)
        .cloned()
        .collect::<Vec<_>>();

    info!(
        new_peers = new_peers.len(),
        total_known_peer = total_known_peers.len(),
        "Storing peers to file"
    );

    let peers = [total_known_peers, new_peers].concat();

    tokio::fs::write(
        "peers.json",
        serde_json::to_string_pretty(&peers).expect("Failed to serialize peers to JSON"),
    )
    .await
    .expect("Failed to write peers to file");
}
