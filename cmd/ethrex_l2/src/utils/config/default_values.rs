use ethereum_types::{H160, H256};

pub const DEFAULT_CONFIG_NAME: &str = "local";
pub const DEFAULT_L1_RPC_URL: &str = "http://localhost:8545";
pub const DEFAULT_L1_CHAIN_ID: u64 = 3151908;
pub const DEFAULT_L2_RPC_URL: &str = "http://localhost:1729";
pub const DEFAULT_L2_CHAIN_ID: u64 = 1729;
pub const DEFAULT_L2_EXPLORER_URL: &str = "";
pub const DEFAULT_L1_EXPLORER_URL: &str = "";
//0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924
pub const DEFAULT_PRIVATE_KEY: H256 = H256([
    0x38, 0x5c, 0x54, 0x64, 0x56, 0xb6, 0xa6, 0x03, 0xa1, 0xcf, 0xca, 0xa9, 0xec, 0x94, 0x94, 0xba,
    0x48, 0x32, 0xda, 0x08, 0xdd, 0x6b, 0xcf, 0x4d, 0xe9, 0xa7, 0x1e, 0x4a, 0x01, 0xb7, 0x49, 0x24,
]);
// 0x3d1e15a1a55578f7c920884a9943b3b35d0d885b
pub const DEFAULT_ADDRESS: H160 = H160([
    0x3d, 0x1e, 0x15, 0xa1, 0xa5, 0x55, 0x78, 0xf7, 0xc9, 0x20, 0x88, 0x4a, 0x99, 0x43, 0xb3, 0xb3,
    0x5d, 0x0d, 0x88, 0x5b,
]);
// 0xB5C064F59b03692361C3750D6d2118B5CfA1Cf91
pub const DEFAULT_CONTRACTS_COMMON_BRIDGE_ADDRESS: H160 = H160([
    0xB5, 0xC0, 0x64, 0xF5, 0x9b, 0x03, 0x69, 0x23, 0x61, 0xC3, 0x75, 0x0D, 0x6d, 0x21, 0x18, 0xB5,
    0xcf, 0xA1, 0xcf, 0x91,
]);
// 0xe9927d77c931f8648da4cc6751ef4e5e2ce74608
pub const DEFAULT_CONTRACTS_ON_CHAIN_PROPOSER_ADDRESS: H160 = H160([
    0xe9, 0x92, 0x7d, 0x77, 0xc9, 0x31, 0xf8, 0x64, 0x8d, 0xa4, 0xcc, 0x67, 0x51, 0xef, 0x4e, 0x5e,
    0x2c, 0xe7, 0x46, 0x08,
]);
