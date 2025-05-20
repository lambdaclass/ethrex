use ethrex_common::{H256, U256};
use ethrex_l2_sdk::calldata::Value;
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Display};

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
    /// It has to match the `OnChainProposer.sol` verifyBatch() function
    pub fn empty_calldata(&self) -> Vec<Value> {
        match self {
            ProverType::RISC0 => {
                vec![
                    Value::Bytes(vec![].into()), // risc0_seal
                    Value::FixedBytes(H256::zero().to_fixed_bytes().to_vec().into()), // image_id
                ]
            }
            ProverType::SP1 => {
                vec![
                    Value::FixedBytes(H256::zero().to_fixed_bytes().to_vec().into()), // sp1_vkey
                    Value::Bytes(vec![].into()),                                      // proof_bytes
                ]
            }
            ProverType::Pico => {
                vec![
                    Value::FixedBytes(H256::zero().to_fixed_bytes().to_vec().into()), // pico_riscv_vkey
                    Value::FixedArray(vec![Value::Uint(U256::zero()); 8]),            // proof
                ]
            }
            ProverType::Exec => unimplemented!("Doesn't need to generate an empty calldata."),
        }
    }

    pub fn verifier_getter(&self) -> Option<String> {
        // These values have to match with the OnChainProposer.sol contract
        match self {
            Self::RISC0 => Some("R0VERIFIER()".to_string()),
            Self::SP1 => Some("SP1VERIFIER()".to_string()),
            Self::Pico => Some("PICOVERIFIER()".to_string()),
            Self::Exec => None,
        }
    }
}

impl Display for ProverType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exec => write!(f, "Exec"),
            Self::RISC0 => write!(f, "RISC0"),
            Self::SP1 => write!(f, "SP1"),
            Self::Pico => write!(f, "Pico"),
        }
    }
}

/// Contains the data ready to be sent to the on-chain verifiers.
#[derive(PartialEq, Serialize, Deserialize, Clone, Debug)]
pub struct ProofCalldata {
    pub prover_type: ProverType,
    pub calldata: Vec<Value>,
}
