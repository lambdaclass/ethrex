// Re-export input/output types from the appropriate guest crate
#[cfg(feature = "l2")]
pub use ethrex_guest_l2::{L2ExecutionError, ProgramInput, ProgramOutput, execution_program};
#[cfg(not(feature = "l2"))]
pub use ethrex_guest_program::{ExecutionError, ProgramInput, ProgramOutput, execution_program};

// When running clippy, the ELFs are not built, so we define them empty.

#[cfg(all(not(clippy), feature = "sp1"))]
pub static ZKVM_SP1_PROGRAM_ELF: &[u8] =
    include_bytes!("../../../ethrex-guest/bin/sp1/out/riscv32im-succinct-zkvm-elf");
#[cfg(any(clippy, not(feature = "sp1")))]
pub const ZKVM_SP1_PROGRAM_ELF: &[u8] = &[];

#[cfg(all(not(clippy), feature = "risc0"))]
pub static ZKVM_RISC0_PROGRAM_VK: &str = include_str!(concat!(
    "../../../ethrex-guest/bin/risc0/out/riscv32im-risc0-vk"
));
#[cfg(any(clippy, not(feature = "risc0")))]
pub const ZKVM_RISC0_PROGRAM_VK: &str = "";

#[cfg(all(not(clippy), feature = "zisk"))]
pub static ZKVM_ZISK_PROGRAM_ELF: &[u8] = include_bytes!(
    "../../../ethrex-guest/bin/zisk/target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk"
);
#[cfg(any(clippy, not(feature = "zisk")))]
pub const ZKVM_ZISK_PROGRAM_ELF: &[u8] = &[];

// RISC0 methods stub (ELF and image ID are embedded by risc0-build)
#[cfg(any(clippy, not(feature = "risc0")))]
pub const ZKVM_RISC0_PROGRAM_ELF: &[u8] = &[0];
#[cfg(any(clippy, not(feature = "risc0")))]
pub const ZKVM_RISC0_PROGRAM_ID: [u32; 8] = [0_u32; 8];
#[cfg(all(not(clippy), feature = "risc0"))]
include!(concat!(env!("OUT_DIR"), "/methods.rs"));
