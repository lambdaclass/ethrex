use serde::{Deserialize, Serialize};
use std::{
    fmt::{Debug, Display},
};

use crate::calldata::Value;

/// Enum used to identify the different proving systems.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProverType {
    Exec,
    RISC0,
    SP1,
    TDX,
}

impl From<ProverType> for u32 {
    fn from(value: ProverType) -> u32 {
        match value {
            ProverType::Exec => 0,
            ProverType::RISC0 => 1,
            ProverType::SP1 => 2,
            ProverType::TDX => 3,
        }
    }
}

impl ProverType {
    /// Used to iterate through all the possible proving systems
    pub fn all() -> impl Iterator<Item = ProverType> {
        [
            ProverType::Exec,
            ProverType::RISC0,
            ProverType::SP1,
            ProverType::TDX,
        ]
        .into_iter()
    }

    /// Used to get the empty_calldata structure for that specific prover
    /// It has to match the `OnChainProposer.sol` verify() function
    pub fn empty_calldata(&self) -> Vec<Value> {
        match self {
            ProverType::RISC0 => {
                vec![Value::Bytes(vec![].into()), Value::Bytes(vec![].into())]
            }
            ProverType::SP1 => {
                vec![Value::Bytes(vec![].into()), Value::Bytes(vec![].into())]
            }
            ProverType::TDX => {
                vec![Value::Bytes(vec![].into()), Value::Bytes(vec![].into())]
            }
            ProverType::Exec => unimplemented!("Doesn't need to generate an empty calldata."),
        }
    }

    pub fn verifier_getter(&self) -> Option<String> {
        // These values have to match with the OnChainProposer.sol contract
        match self {
            Self::RISC0 => Some("REQUIRE_RISC0_PROOF()".to_string()),
            Self::SP1 => Some("REQUIRE_SP1_PROOF()".to_string()),
            Self::TDX => Some("REQUIRE_TDX_PROOF()".to_string()),
            Self::Exec => None,
        }
    }

    pub fn aligned_vm_program_code(&self) -> std::io::Result<Option<Vec<u8>>> {
        // TODO: these should be compile-time consts
        let path = match self {
            // for risc0, Aligned requires the image id
            Self::RISC0 => format!(
                "{}/../prover/zkvm/interface/risc0/out/riscv32im-risc0-zkvm-vk",
                env!("CARGO_MANIFEST_DIR")
            ),
            // for sp1, Aligned requires the ELF file
            Self::SP1 => format!(
                "{}/../prover/zkvm/interface/sp1/out/riscv32im-succinct-zkvm-elf",
                env!("CARGO_MANIFEST_DIR")
            ),
            // other types are not supported by Aligned
            _ => return Ok(None),
        };
        let path = std::fs::canonicalize(path)?;
        std::fs::read(path).map(Some)
    }

    pub fn vk(&self, aligned: bool) -> std::io::Result<Option<Vec<u8>>> {
        // TODO: these should be compile-time consts
        let path = match &self {
            Self::RISC0 => format!(
                "{}/../prover/zkvm/interface/risc0/out/riscv32im-risc0-vk",
                env!("CARGO_MANIFEST_DIR")
            ),
            // Aligned requires the vk's 32 bytes hash, while the L1 verifier requires
            // the hash as a bn254 F_r element.
            Self::SP1 if aligned => format!(
                "{}/../prover/zkvm/interface/sp1/out/riscv32im-succinct-zkvm-vk-u32",
                env!("CARGO_MANIFEST_DIR")
            ),
            Self::SP1 if !aligned => format!(
                "{}/../prover/zkvm/interface/sp1/out/riscv32im-succinct-zkvm-vk-bn254",
                env!("CARGO_MANIFEST_DIR")
            ),
            // other types don't have a verification key
            _ => return Ok(None),
        };
        let path = std::fs::canonicalize(path)?;
        std::fs::read(path).map(Some)
    }
}

impl Display for ProverType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exec => write!(f, "Exec"),
            Self::RISC0 => write!(f, "RISC0"),
            Self::SP1 => write!(f, "SP1"),
            Self::TDX => write!(f, "TDX"),
        }
    }
}

/// Contains the proof data recently created by the prover.
/// It can be either a `ProofCalldata` ready to be sent to the on-chain verifier or a `ProofBytes`
/// to be sent to Aligned.
#[derive(PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum BatchProof {
    ProofCalldata(ProofCalldata),
    ProofBytes(ProofBytes),
}

impl BatchProof {
    pub fn prover_type(&self) -> ProverType {
        match self {
            BatchProof::ProofCalldata(proof) => proof.prover_type,
            BatchProof::ProofBytes(proof) => proof.prover_type,
        }
    }

    pub fn calldata(&self) -> Vec<Value> {
        match self {
            BatchProof::ProofCalldata(proof) => proof.calldata.clone(),
            BatchProof::ProofBytes(_) => vec![],
        }
    }

    pub fn compressed(&self) -> Option<Vec<u8>> {
        match self {
            BatchProof::ProofCalldata(_) => None,
            BatchProof::ProofBytes(proof) => Some(proof.proof.clone()),
        }
    }

    pub fn public_values(&self) -> Vec<u8> {
        match self {
            BatchProof::ProofCalldata(_) => vec![],
            BatchProof::ProofBytes(proof_bytes) => proof_bytes.public_values.clone(),
        }
    }
}

/// Contains the Proof and the public values generated by the prover.
/// It is used to send the proof to Aligned.
#[derive(PartialEq, Serialize, Deserialize, Clone, Debug)]
pub struct ProofBytes {
    pub prover_type: ProverType,
    pub proof: Vec<u8>,
    pub public_values: Vec<u8>,
}

/// Contains the data ready to be sent to the on-chain verifiers.
#[derive(PartialEq, Serialize, Deserialize, Clone, Debug)]
pub struct ProofCalldata {
    pub prover_type: ProverType,
    pub calldata: Vec<Value>,
}

/// Indicates the prover which proof *format* to generate
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub enum ProofFormat {
    #[default]
    /// A compressed proof wrapped over groth16. EVM friendly.
    Groth16,
    /// Fixed size STARK execution proof.
    Compressed,
}
