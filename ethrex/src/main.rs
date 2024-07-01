use ethrex_core::types::Genesis;
use ethrex_net::types::BootNode;
use std::{
    io::{self, BufReader},
    net::{SocketAddr, ToSocketAddrs},
    str::FromStr,
};
use tokio::try_join;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;
mod cli;

#[tokio::main]
async fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let matches = cli::cli().get_matches();

    let http_addr = matches
        .get_one::<String>("http.addr")
        .expect("http.addr is required");
    let http_port = matches
        .get_one::<String>("http.port")
        .expect("http.port is required");
    let authrpc_addr = matches
        .get_one::<String>("authrpc.addr")
        .expect("authrpc.addr is required");
    let authrpc_port = matches
        .get_one::<String>("authrpc.port")
        .expect("authrpc.port is required");

    let tcp_addr = matches
        .get_one::<String>("p2p.addr")
        .expect("addr is required");
    let tcp_port = matches
        .get_one::<String>("p2p.port")
        .expect("port is required");
    let udp_addr = matches
        .get_one::<String>("discovery.addr")
        .expect("discovery.addr is required");
    let udp_port = matches
        .get_one::<String>("discovery.port")
        .expect("discovery.port is required");

    let genesis_file_path = matches
        .get_one::<String>("network")
        .expect("network is required");

    let bootnode_list: Vec<_> = matches
        .get_many::<String>("bootnodes")
        .expect("bootnodes is required")
        .collect();

    let _bootnodes: Vec<BootNode> = bootnode_list
        .iter()
        .map(|s| BootNode::from_str(s).expect("Failed to parse bootnodes"))
        .collect();

    let http_socket_addr =
        parse_socket_addr(http_addr, http_port).expect("Failed to parse http address and port");
    let authrpc_socket_addr = parse_socket_addr(authrpc_addr, authrpc_port)
        .expect("Failed to parse authrpc address and port");

    let udp_socket_addr =
        parse_socket_addr(udp_addr, udp_port).expect("Failed to parse discovery address and port");
    let tcp_socket_addr =
        parse_socket_addr(tcp_addr, tcp_port).expect("Failed to parse addr and port");

    let _genesis = read_genesis_file(genesis_file_path);

    let rpc_api = ethrex_rpc::start_api(http_socket_addr, authrpc_socket_addr);
    let networking = ethrex_net::start_network(udp_socket_addr, tcp_socket_addr);

    try_join!(tokio::spawn(rpc_api), tokio::spawn(networking)).unwrap();
}

fn read_genesis_file(genesis_file_path: &str) -> Genesis {
    let genesis_file = std::fs::File::open(genesis_file_path).expect("Failed to open genesis file");
    let genesis_reader = BufReader::new(genesis_file);
    serde_json::from_reader(genesis_reader).expect("Failed to read genesis file")
}

fn parse_socket_addr(addr: &str, port: &str) -> io::Result<SocketAddr> {
    // NOTE: this blocks until hostname can be resolved
    format!("{addr}:{port}")
        .to_socket_addrs()?
        .next()
        .ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Failed to parse socket address",
        ))
}
