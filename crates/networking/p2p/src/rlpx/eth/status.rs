pub use super::eth68::status::StatusMessage68;
pub use super::eth69::status::StatusMessage69;
use ethrex_common::types::{BlockHash, ForkId};

pub trait StatusMessage {
    fn get_network_id(&self) -> u64;

    fn get_eth_version(&self) -> u8;

    fn get_fork_id(&self) -> ForkId;

    fn get_genesis(&self) -> BlockHash;
}
