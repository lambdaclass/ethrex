use crate::Address;
use std::{str::FromStr, sync::LazyLock};

use crate::types::Fork::{self, *};

pub struct SystemContract<'a> {
    pub address: Address,
    pub name: &'a str,
    pub active_since_fork: Fork,
}

pub static SYSTEM_ADDRESS: LazyLock<SystemContract> = LazyLock::new(|| SystemContract {
    address: Address::from_str("fffffffffffffffffffffffffffffffffffffffe")
        .expect("Failed to get address from string"),
    name: "SYSTEM_ADDRESS",
    active_since_fork: Paris,
});
pub static BEACON_ROOTS_ADDRESS: LazyLock<SystemContract> = LazyLock::new(|| SystemContract {
    address: Address::from_str("000F3df6D732807Ef1319fB7B8bB8522d0Beac02")
        .expect("Failed to get address from string"),
    name: "BEACON_ROOTS_ADDRESS",
    active_since_fork: Prague,
});
pub static HISTORY_STORAGE_ADDRESS: LazyLock<SystemContract> = LazyLock::new(|| SystemContract {
    address: Address::from_str("0000F90827F1C53a10cb7A02335B175320002935")
        .expect("Failed to get address from string"),
    name: "HISTORY_STORAGE_ADDRESS",
    active_since_fork: Prague,
});
pub static WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS: LazyLock<SystemContract> =
    LazyLock::new(|| SystemContract {
        address: Address::from_str("00000961Ef480Eb55e80D19ad83579A64c007002")
            .expect("Failed to get address from string"),
        name: "WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS",
        active_since_fork: Prague,
    });
pub static CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS: LazyLock<SystemContract> =
    LazyLock::new(|| SystemContract {
        address: Address::from_str("0000BBdDc7CE488642fb579F8B00f3a590007251")
            .expect("Failed to get address from string"),
        name: "CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS",
        active_since_fork: Prague,
    });

pub const SYSTEM_CONTRACTS: [&LazyLock<SystemContract>; 5] = [
    &SYSTEM_ADDRESS,
    &BEACON_ROOTS_ADDRESS,
    &HISTORY_STORAGE_ADDRESS,
    &WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
    &CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
];
