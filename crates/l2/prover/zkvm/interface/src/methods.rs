#[cfg(any(clippy, not(feature = "risc0")))]
pub const ZKVM_RISC0_PROGRAM_ELF: &[u8] = &[0];
#[cfg(any(clippy, not(feature = "risc0")))]
pub const ZKVM_RISC0_PROGRAM_ID: [u32; 8] = [0_u32; 8];
#[cfg(all(not(clippy), feature = "risc0"))]
include!(concat!(env!("OUT_DIR"), "/methods.rs"));

#[cfg(all(not(clippy), feature = "pico"))]
pub const ZKVM_PICO_PROGRAM_ELF: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/riscv32im-pico-zkvm-elf"));
#[cfg(any(clippy, not(feature = "pico")))]
pub const ZKVM_PICO_PROGRAM_ELF: &[u8] = &[0];
