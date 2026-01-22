#[cfg(any(clippy, not(feature = "risc0")))]
pub const ETHREX_GUEST_RISC0_ELF: &[u8] = &[0];
#[cfg(any(clippy, not(feature = "risc0")))]
pub const ETHREX_GUEST_RISC0_ID: [u32; 8] = [0_u32; 8];

#[cfg(all(not(clippy), feature = "risc0"))]
include!(concat!(env!("OUT_DIR"), "/methods.rs"));
