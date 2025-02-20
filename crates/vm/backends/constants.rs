use ethrex_common::Address;
use std::{str::FromStr, sync::LazyLock};

pub static SYSTEM_ADDRESS: LazyLock<Address> =
    LazyLock::new(|| Address::from_str("fffffffffffffffffffffffffffffffffffffffe").unwrap());
pub static BEACON_ROOTS_ADDRESS: LazyLock<Address> =
    LazyLock::new(|| Address::from_str("000F3df6D732807Ef1319fB7B8bB8522d0Beac02").unwrap());
pub static HISTORY_STORAGE_ADDRESS: LazyLock<Address> =
    LazyLock::new(|| Address::from_str("0000F90827F1C53a10cb7A02335B175320002935").unwrap());
pub static WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS: LazyLock<Address> =
    LazyLock::new(|| Address::from_str("00000961Ef480Eb55e80D19ad83579A64c007002").unwrap());
pub static CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS: LazyLock<Address> =
    LazyLock::new(|| Address::from_str("0000BBdDc7CE488642fb579F8B00f3a590007251").unwrap());
