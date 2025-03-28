pub mod initializers;
pub mod utils;

pub mod cli;

#[cfg(any(feature = "l2", feature = "based"))]
pub mod l2;

mod decode;
mod networks;

pub const DEFAULT_DATADIR: &str = "ethrex";
pub const DEFAULT_L2_DATADIR: &str = "ethrex-l2";
