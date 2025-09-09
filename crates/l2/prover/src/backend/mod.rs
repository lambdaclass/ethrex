use std::str::FromStr;

use clap::ValueEnum;
use guest_program::output::ProgramOutput;
use serde::{Deserialize, Serialize};

pub mod exec;

#[cfg(feature = "risc0")]
pub mod risc0;

#[cfg(feature = "sp1")]
pub mod sp1;

#[cfg(feature = "openvm")]
pub mod openvm;

#[derive(Default, Debug, Deserialize, Serialize, Copy, Clone, ValueEnum)]
pub enum Backend {
    #[default]
    Exec,
    #[cfg(feature = "sp1")]
    SP1,
    #[cfg(feature = "risc0")]
    RISC0,
    #[cfg(feature = "openvm")]
    OpenVM,
}

// Needed for Clap
impl FromStr for Backend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "exec" => Ok(Backend::Exec),
            #[cfg(feature = "sp1")]
            "sp1" => Ok(Backend::SP1),
            #[cfg(feature = "risc0")]
            "risc0" => Ok(Backend::RISC0),
            #[cfg(feature = "openvm")]
            "openvm" => Ok(Backend::OpenVM),
            _ => Err(Self::Err::from("Invalid backend")),
        }
    }
}

pub enum ProveOutput {
    Exec(ProgramOutput),
    #[cfg(feature = "sp1")]
    SP1(sp1::ProveOutput),
    #[cfg(feature = "risc0")]
    RISC0(risc0_zkvm::Receipt),
    #[cfg(feature = "openvm")]
    OpenVM(openvm::ProgramOutput),
}
