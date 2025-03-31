pub(crate) mod discv4;
pub mod kademlia;
pub mod network;
pub mod peer_handler;
pub mod rlpx;
pub(crate) mod snap;
pub mod sync;
pub mod types;

pub use network::periodically_show_peer_stats;
pub use network::start_network;

pub mod constants {
    include!(concat!(env!("OUT_DIR"), "/constants.rs"));
}
