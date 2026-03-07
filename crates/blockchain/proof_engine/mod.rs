#[cfg(feature = "eip-8025")]
pub mod config;
#[cfg(feature = "eip-8025")]
pub mod ssz;
#[cfg(feature = "eip-8025")]
pub mod types;

#[cfg(feature = "eip-8025")]
mod engine;
#[cfg(feature = "eip-8025")]
pub use engine::ProofEngine;
