// ELF and VK constants for zkVM backends
// These are used by the l1_proof_sender when submitting proofs to Aligned

#[cfg(all(not(clippy), feature = "sp1"))]
pub static ZKVM_SP1_PROGRAM_ELF: &[u8] =
    include_bytes!("../../ethrex-guest/bin/sp1/out/riscv32im-succinct-zkvm-elf");
#[cfg(any(clippy, not(feature = "sp1")))]
pub const ZKVM_SP1_PROGRAM_ELF: &[u8] = &[];

#[cfg(all(not(clippy), feature = "risc0"))]
pub static ZKVM_RISC0_PROGRAM_VK: &str = include_str!(concat!(
    "../../ethrex-guest/bin/risc0/out/riscv32im-risc0-vk"
));
#[cfg(any(clippy, not(feature = "risc0")))]
pub const ZKVM_RISC0_PROGRAM_VK: &str = "";
