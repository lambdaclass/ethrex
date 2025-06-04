pub mod initializers;
pub mod utils;

pub mod cli;

#[cfg(feature = "l2")]
pub mod l2;

mod decode;
pub mod networks;

pub const DEFAULT_DATADIR: &str = "ethrex";
pub const DEFAULT_L2_DATADIR: &str = "ethrex-l2";
pub const DEFAULT_CUSTOM_DIR: &str = "custom";
pub const DEFAULT_STORE_DIR: &str = "store";
pub const DEFAULT_JWT_PATH: &str = "jwt.hex";
