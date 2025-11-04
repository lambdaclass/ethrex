use ethereum_types::U256;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BalanceDiff {
    pub chain_id: U256,
    pub value: U256,
}
