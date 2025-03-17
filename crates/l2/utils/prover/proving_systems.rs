use ethrex_l2_sdk::calldata::Value;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// Enum used to identify the different proving systems.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProverType {
    Exec,
    RISC0,
    SP1,
    Pico,
}

impl ProverType {
    /// Used to iterate through all the possible proving systems
    pub fn all() -> impl Iterator<Item = ProverType> {
        [
            ProverType::Exec,
            ProverType::RISC0,
            ProverType::SP1,
            ProverType::Pico,
        ]
        .into_iter()
    }
}

/// Contains the data ready to be sent to the on-chain verifiers.
#[derive(PartialEq, Serialize, Deserialize, Clone, Debug)]
pub struct ProofCalldata {
    pub prover_type: ProverType,
    pub calldata: Vec<Value>,
}
