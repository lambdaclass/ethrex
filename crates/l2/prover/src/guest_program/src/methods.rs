#[cfg(any(clippy, not(feature = "risc0-guest")))]
pub const ZKVM_RISC0_PROGRAM_ELF: &[u8] = &[0];
#[cfg(any(clippy, not(feature = "risc0-guest")))]
pub const ZKVM_RISC0_PROGRAM_ID: [u32; 8] = [0_u32; 8];
#[cfg(all(not(clippy), feature = "risc0-guest"))]
include!(concat!(env!("OUT_DIR"), "/methods.rs"));
