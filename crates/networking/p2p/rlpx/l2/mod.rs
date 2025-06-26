use super::p2p::Capability;

pub const SUPPORTED_BASED_CAPABILITIES: [Capability; 1] = [Capability::based(1)];
pub mod l2_connection;
pub mod messages;
