use ethrex_common::{H160, types::Fork, types::Fork::*};

pub struct SystemContract<'a> {
    pub address: H160,
    pub name: &'a str,
    pub active_since_fork: Fork,
}

pub const SYSTEM_ADDRESS: SystemContract<'_> = SystemContract {
    address: H160([
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xfe,
    ]),
    name: "SYSTEM_ADDRESS",
    active_since_fork: Paris,
};

pub const BEACON_ROOTS_ADDRESS: SystemContract<'_> = SystemContract {
    address: H160([
        0x00, 0x0F, 0x3d, 0xf6, 0xD7, 0x32, 0x80, 0x7E, 0xf1, 0x31, 0x9f, 0xB7, 0xB8, 0xbB, 0x85,
        0x22, 0xd0, 0xBe, 0xac, 0x02,
    ]),
    name: "BEACON_ROOTS_ADDRESS",
    active_since_fork: Prague,
};

pub const HISTORY_STORAGE_ADDRESS: SystemContract<'_> = SystemContract {
    address: H160([
        0x00, 0x00, 0xF9, 0x08, 0x27, 0xF1, 0xC5, 0x3a, 0x10, 0xcb, 0x7A, 0x02, 0x33, 0x5B, 0x17,
        0x53, 0x20, 0x00, 0x29, 0x35,
    ]),
    name: "HISTORY_STORAGE_ADDRESS",
    active_since_fork: Prague,
};

pub const WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS: SystemContract<'_> = SystemContract {
    address: H160([
        0x00, 0x00, 0x09, 0x61, 0xEf, 0x48, 0x0E, 0xb5, 0x5e, 0x80, 0xD1, 0x9a, 0xd8, 0x35, 0x79,
        0xA6, 0x4c, 0x00, 0x70, 0x02,
    ]),
    name: "WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS",
    active_since_fork: Prague,
};

pub const CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS: SystemContract<'_> = SystemContract {
    address: H160([
        0x00, 0x00, 0xBB, 0xdD, 0xc7, 0xCE, 0x48, 0x86, 0x42, 0xfb, 0x57, 0x9F, 0x8B, 0x00, 0xf3,
        0xa5, 0x90, 0x00, 0x72, 0x51,
    ]),
    name: "CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS",
    active_since_fork: Prague,
};

pub const SYSTEM_CONTRACTS: [SystemContract<'_>; 5] = [
    SYSTEM_ADDRESS,
    BEACON_ROOTS_ADDRESS,
    HISTORY_STORAGE_ADDRESS,
    WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
    CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
];

pub fn system_contracts_for_fork(fork: Fork) -> impl Iterator<Item = SystemContract<'static>> {
    SYSTEM_CONTRACTS
        .into_iter()
        .filter(move |system_contract| system_contract.active_since_fork <= fork)
}
