pub mod common;
pub mod crypto;
pub mod l1;
pub mod l2;
pub mod methods;
pub mod scopes;

// Backward-compatible re-exports based on feature flag.
// The prover backend uses `ethrex_guest_program::input::ProgramInput`, etc.
// These re-exports allow existing code to work without changes.

#[cfg(feature = "l2")]
pub mod input {
    pub use crate::l2::ProgramInput;
}
#[cfg(not(feature = "l2"))]
pub mod input {
    pub use crate::l1::ProgramInput;
}

#[cfg(feature = "l2")]
pub mod output {
    pub use crate::l2::ProgramOutput;
}
#[cfg(not(feature = "l2"))]
pub mod output {
    pub use crate::l1::ProgramOutput;
}

#[cfg(feature = "l2")]
pub mod execution {
    pub use crate::l2::execution_program;
}
#[cfg(not(feature = "l2"))]
pub mod execution {
    pub use crate::l1::execution_program;
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
    include_bytes!("../bin/zisk/target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk");
#[cfg(any(clippy, not(feature = "zisk-build-elf")))]
pub const ZKVM_ZISK_PROGRAM_ELF: &[u8] = &[];

/// Report cycles used in a code block when running inside a zkVM.
///
/// Each call is tagged with a compile-time scope ID (from the `scopes` module)
/// that identifies the logical phase being measured.
///
/// When `sp1-cycles` is enabled, prints SP1-compatible cycle tracking messages.
/// When `zisk-scopes` is enabled, emits ZisK AIR-cost profiling scope markers.
/// When neither is enabled, executes the closure with no overhead.
pub fn report_cycles<const SCOPE: u16, T, E>(
    _label: &str,
    block: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    #[cfg(feature = "zisk-scopes")]
    ziskos::ziskos_profile_start::<SCOPE>();
    #[cfg(feature = "sp1-cycles")]
    println!("cycle-tracker-report-start: {_label}");
    let result = block();
    #[cfg(feature = "sp1-cycles")]
    println!("cycle-tracker-report-end: {_label}");
    #[cfg(feature = "zisk-scopes")]
    ziskos::ziskos_profile_end::<SCOPE>();
    result
}
