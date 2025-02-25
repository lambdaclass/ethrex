#[cfg(not(feature = "l2"))]
pub mod l1;
#[cfg(feature = "l2")]
pub mod l2;

pub const DEFAULT_DATADIR: &str = "ethrex";
