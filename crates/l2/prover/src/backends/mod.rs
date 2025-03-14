#[cfg(not(any(feature = "exec", feature = "pico", feature = "risc0", feature = "sp1")))]
compile_error!("A prover backend must be chosen by enabling one of the next features: exec, pico, risc0, sp1.");

#[cfg(feature = "exec")]
pub mod exec;

#[cfg(feature = "pico")]
pub mod pico;

#[cfg(feature = "risc0")]
pub mod risc0;

#[cfg(feature = "sp1")]
pub mod sp1;
