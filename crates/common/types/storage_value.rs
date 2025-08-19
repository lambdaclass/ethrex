use ethereum_types::U256;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageValue {
    pub current_value: U256,
    // Whether the slot is cold can be inferred from the type of previous value
    // If its Some(U256) then it's not cold
    // But if it's none if means the slot wasn't accessed in the current transaction
    pub previous_value: Option<U256>,
}
