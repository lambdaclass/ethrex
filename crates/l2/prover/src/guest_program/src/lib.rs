pub mod execution;
pub mod input;
pub mod methods;
pub mod output;

// When running clippy, the ELFs are not built, so we define them empty.

#[cfg(all(not(clippy), feature = "sp1"))]
pub static ZKVM_SP1_PROGRAM_ELF: &[u8] = include_bytes!("./sp1/out/riscv32im-succinct-zkvm-elf");
#[cfg(any(clippy, not(feature = "sp1")))]
pub const ZKVM_SP1_PROGRAM_ELF: &[u8] = &[];

#[cfg(all(not(clippy), feature = "risc0"))]
pub static ZKVM_RISC0_PROGRAM_VK: &str = include_str!(concat!("./risc0/out/riscv32im-risc0-vk"));
#[cfg(any(clippy, not(feature = "risc0")))]
pub const ZKVM_RISC0_PROGRAM_VK: &str = "";

#[cfg(all(not(clippy), feature = "zisk"))]
pub static ZKVM_ZISK_PROGRAM_ELF: &[u8] =
    include_bytes!("./zisk/target/riscv64ima-zisk-zkvm-elf/release/zkvm-zisk-program");
#[cfg(any(clippy, not(feature = "zisk")))]
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
