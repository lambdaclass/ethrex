#[cfg(any(clippy, not(feature = "risc0")))]
pub const ZKVM_RISC0_PROGRAM_ELF: &[u8] = &[0];
#[cfg(any(clippy, not(feature = "risc0")))]
pub const ZKVM_RISC0_PROGRAM_ID: [u32; 8] = [0_u32; 8];

// Include risc0-generated methods.rs which provides ETHREX_GUEST_RISC0_ELF and ETHREX_GUEST_RISC0_ID
#[cfg(all(not(clippy), feature = "risc0"))]
include!(concat!(env!("OUT_DIR"), "/methods.rs"));

// Re-export with backward-compatible names expected by the prover
#[cfg(all(not(clippy), feature = "risc0"))]
pub const ZKVM_RISC0_PROGRAM_ELF: &[u8] = ETHREX_GUEST_RISC0_ELF;
#[cfg(all(not(clippy), feature = "risc0"))]
pub const ZKVM_RISC0_PROGRAM_ID: [u32; 8] = ETHREX_GUEST_RISC0_ID;
