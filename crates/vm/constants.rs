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

// transactions_root(H256) + receipts_root(H256) + gas_limit(u64) + gas_used(u64) + timestamp(u64) + base_fee_per_gas(u64).
// 32bytes + 32bytes + 8bytes + 8bytes + 8bytes + 8bytes
pub const HEADER_FIELDS_SIZE: usize = 96;

// address(H160) + amount(U256) + tx_hash(H256).
// 20bytes + 32bytes + 32bytes.
pub const L2_WITHDRAWAL_SIZE: usize = 84;

// address(H160) + amount(U256).
// 20bytes + 32bytes
pub const L2_DEPOSIT_SIZE: usize = 52;

pub static COMMON_BRIDGE_L2_ADDRESS: LazyLock<Address> = LazyLock::new(|| {
    Address::from_slice(&[
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0xff, 0xff,
    ])
});
