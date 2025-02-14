use ethrex_common::Address;
use std::{str::FromStr, sync::LazyLock};

pub static SYSTEM_ADDRESS: LazyLock<Address> =
    LazyLock::new(|| Address::from_str("fffffffffffffffffffffffffffffffffffffffe").unwrap());
pub static BEACON_ROOTS_ADDRESS: LazyLock<Address> =
    LazyLock::new(|| Address::from_str("000F3df6D732807Ef1319fB7B8bB8522d0Beac02").unwrap());
pub static HISTORY_STORAGE_ADDRESS: LazyLock<Address> =
    LazyLock::new(|| Address::from_str("0000F90827F1C53a10cb7A02335B175320002935").unwrap());
