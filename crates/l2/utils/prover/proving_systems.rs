use ethrex_common::{Bytes, H256, U256};
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

    /// Used to get the empty_calldata structure for that specific prover
    /// It has to match the `OnChainProposer.sol` verify() function
    pub fn empty_calldata(&self) -> Vec<Value> {
        match self {
            ProverType::RISC0 => {
                vec![
                    Value::Bytes(vec![].into()),
                    Value::FixedBytes(H256::zero().to_fixed_bytes().to_vec().into()),
                    Value::FixedBytes(H256::zero().to_fixed_bytes().to_vec().into()),
                ]
            }
            ProverType::SP1 => {
                vec![
                    Value::FixedBytes(H256::zero().to_fixed_bytes().to_vec().into()),
                    Value::Bytes(vec![].into()),
                    Value::Bytes(vec![].into()),
                ]
            }
            ProverType::Pico => {
                vec![
                    Value::FixedBytes(H256::zero().as_bytes().to_vec().into()),
                    Value::Bytes(vec![].into()),
                    Value::FixedArray(vec![Value::Uint(U256::zero()); 8]),
                ]
            }
            ProverType::Exec => unimplemented!("Doesn't need to generate an empty calldata."),
        }
    }
}

/// Contains the data ready to be sent to the on-chain verifiers.
#[derive(PartialEq, Serialize, Deserialize, Clone, Debug)]
pub struct ProofCalldata {
    pub prover_type: ProverType,
    pub calldata: Vec<Value>,
    pub vkey: Bytes,
}
