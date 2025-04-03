use ethrex_common::U256;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageSlot {
    pub original_value: U256,
    pub current_value: U256,
}
