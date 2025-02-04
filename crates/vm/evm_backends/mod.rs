pub mod levm;
pub mod revm;

use crate::errors::EvmError;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub enum EVM {
    LEVM,
    REVM,
}

impl Default for EVM {
    fn default() -> Self {
        EVM::REVM
    }
}

impl FromStr for EVM {
    type Err = EvmError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "levm" => Ok(EVM::LEVM),
            "revm" => Ok(EVM::REVM),
            _ => Err(EvmError::InvalidEVM(s.to_string())),
        }
    }
}
