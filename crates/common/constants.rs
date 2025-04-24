// === EIP-4844 constants ===

/// Gas consumption of a single data blob (== blob byte size).
pub const GAS_PER_BLOB: u64 = 1 << 17;

// Minimum base fee per blob
pub const MIN_BASE_FEE_PER_BLOB_GAS: u64 = 1;

pub const ETHREX_PKG_NAME: &str = env!("CARGO_PKG_NAME");
pub const ETHREX_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ETHREX_COMMIT_HASH: &str = env!("VERGEN_RUSTC_COMMIT_HASH");
pub const ETHREX_BUILD_OS: &str = env!("VERGEN_RUSTC_HOST_TRIPLE");
pub const ETHREX_RUSTC_VERSION: &str = env!("VERGEN_RUSTC_SEMVER");
