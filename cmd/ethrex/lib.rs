pub mod cli;
pub mod initializers;

#[cfg(feature = "l2")]
pub mod l2;
pub mod utils;

mod decode;
pub mod networks;

pub const DEFAULT_DATADIR: &str = "ethrex";
pub const DEFAULT_L2_DATADIR: &str = "ethrex-l2";
