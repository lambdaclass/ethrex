#![allow(dead_code)]

use bytes::Bytes;
use ethereum_types::{Address, U256};
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, Clone)]
pub struct BlockAccessList {
    inner: Vec<AccountChanges>,
}

#[derive(Default, Serialize, Deserialize, Clone)]
pub struct AccountChanges {
    address: Address,
    slot_changes: Vec<SlotChange>,
    storage_reads: Vec<U256>,
    balance_changes: Vec<(usize, U256)>,
    nonce_changes: Vec<(usize, u64)>,
    code_changes: Vec<(usize, Bytes)>,
}

#[derive(Default, Serialize, Deserialize, Clone)]
pub struct SlotChange {
    slot: U256,
    storage_changes: Vec<(usize, U256)>,
}
