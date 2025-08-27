use super::p2p::Capability;

pub const SUPPORTED_BASED_CAPABILITIES: [Capability; 1] = [Capability::based(1)];
pub const PERIODIC_BLOCK_BROADCAST_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(500);
pub const PERIODIC_BATCH_BROADCAST_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(500);
pub mod l2_connection;
pub mod messages;
