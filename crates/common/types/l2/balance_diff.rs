use ethereum_types::{H256, U256};
use serde::{Deserialize, Serialize};

/// Represents the amount of balance to transfer to the bridge contract for a specific chain.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BalanceDiff {
    pub chain_id: U256,
    pub value: U256,
    pub messasge_hashes: Vec<H256>,
}
