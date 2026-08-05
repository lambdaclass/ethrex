pub mod common;
pub mod crypto;
pub mod l1;
pub mod l2;
pub mod methods;

// Input/output aliases, selected by the `l2` feature.
//
// The L2 batch prover keeps its own rkyv-serialized `ProgramInput` and its own
// commitment shape — the on-chain verifier needs state roots and blob/message
// commitments that `statelessOutputBytes` does not carry.
//
// The L1 guest's input is the spec's `statelessInputBytes`: an opaque blob the
// host passes straight through, aliased here so `ProverBackend` stays
// generic-free. Its output is the spec's `SszStatelessValidationResult`.

#[cfg(feature = "l2")]
pub mod input {
    pub use crate::l2::ProgramInput;
}
#[cfg(not(feature = "l2"))]
pub mod input {
    pub type ProgramInput = Vec<u8>;
}

#[cfg(feature = "l2")]
pub mod output {
    pub use crate::l2::ProgramOutput;
}
#[cfg(not(feature = "l2"))]
pub mod output {
    pub use ethrex_common::types::stateless_ssz::SszStatelessValidationResult as ProgramOutput;
}

#[cfg(feature = "l2")]
pub mod execution {
    pub use crate::l2::execution_program;
}

// When running clippy, the ELFs are not built, so we define them empty.

#[cfg(all(not(clippy), feature = "sp1-build-elf"))]
pub static ZKVM_SP1_PROGRAM_ELF: &[u8] =
    include_bytes!("../bin/sp1/out/riscv32im-succinct-zkvm-elf");
#[cfg(any(clippy, not(feature = "sp1-build-elf")))]
pub const ZKVM_SP1_PROGRAM_ELF: &[u8] = &[];

#[cfg(all(not(clippy), feature = "risc0-build-elf"))]
pub static ZKVM_RISC0_PROGRAM_VK: &str =
    include_str!(concat!("../bin/risc0/out/riscv32im-risc0-vk"));
#[cfg(any(clippy, not(feature = "risc0-build-elf")))]
pub const ZKVM_RISC0_PROGRAM_VK: &str = "";

#[cfg(all(not(clippy), feature = "zisk-build-elf"))]
pub static ZKVM_ZISK_PROGRAM_ELF: &[u8] =
    include_bytes!("../bin/zisk/target/elf/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk");
#[cfg(any(clippy, not(feature = "zisk-build-elf")))]
pub const ZKVM_ZISK_PROGRAM_ELF: &[u8] = &[];

/// Report cycles used in a code block when running inside SP1 zkVM.
///
/// When the feature "sp1-cycles" is enabled, it will print start and end cycle
/// tracking messages that are compatible with SP1's cycle tracking system.
pub fn report_cycles<T, E>(_label: &str, block: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    #[cfg(feature = "sp1-cycles")]
    println!("cycle-tracker-report-start: {_label}");
    let result = block();
    #[cfg(feature = "sp1-cycles")]
    println!("cycle-tracker-report-end: {_label}");
    result
}
