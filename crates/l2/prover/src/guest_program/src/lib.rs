pub mod execution;
pub mod input;
pub mod methods;
pub mod output;

#[cfg(all(not(clippy), feature = "sp1"))]
pub static ZKVM_SP1_PROGRAM_ELF: &[u8] = include_bytes!("./sp1/out/riscv32im-succinct-zkvm-elf");
// If we're running clippy, the file isn't generated.
// To avoid compilation errors, we override it with an empty slice.
#[cfg(any(clippy, not(feature = "sp1")))]
pub const ZKVM_SP1_PROGRAM_ELF: &[u8] = &[];
