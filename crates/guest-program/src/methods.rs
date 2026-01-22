#[cfg(any(clippy, not(feature = "risc0")))]
pub const ZKVM_RISC0_PROGRAM_ELF: &[u8] = &[0];
#[cfg(any(clippy, not(feature = "risc0")))]
pub const ZKVM_RISC0_PROGRAM_ID: [u32; 8] = [0_u32; 8];

// Include our custom risc0_methods.rs which defines ZKVM_RISC0_PROGRAM_ELF and ZKVM_RISC0_PROGRAM_ID
// with the names expected by the prover (for backward compatibility)
#[cfg(all(not(clippy), feature = "risc0"))]
include!(concat!(env!("OUT_DIR"), "/risc0_methods.rs"));
