use bytes::Bytes;
use ethrex_common::types::{
    Block, blobs_bundle, block_execution_witness::ExecutionWitness, fee_config::FeeConfig,
};
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::calldata::Value;

// Re-export prover types from ethrex-common so existing `ethrex_l2_common::prover::X` paths
// continue to work for all downstream crates.
pub use ethrex_common::types::prover::{ProofBytes, ProofFormat, ProverType};

#[serde_as]
#[derive(Serialize, Deserialize, RDeserialize, RSerialize, Archive)]
pub struct ProverInputData {
    pub blocks: Vec<Block>,
    pub execution_witness: ExecutionWitness,
    pub elasticity_multiplier: u64,
    #[serde_as(as = "[_; 48]")]
    pub blob_commitment: blobs_bundle::Commitment,
    #[serde_as(as = "[_; 48]")]
    pub blob_proof: blobs_bundle::Proof,
    pub fee_configs: Vec<FeeConfig>,
}

/// Extension trait for L2-specific `ProverType` methods that depend on the `Value` calldata type.
pub trait ProverTypeL2Ext {
    /// Used to get the empty_calldata structure for that specific prover.
    /// It has to match the `OnChainProposer.sol` verify() function.
    fn empty_calldata(&self) -> Vec<Value>;

    /// Used to call a getter for the REQUIRE_*_PROOF boolean in the OnChainProposer contract.
    fn verifier_getter(&self) -> Option<String>;
}

impl ProverTypeL2Ext for ProverType {
    fn empty_calldata(&self) -> Vec<Value> {
        match self {
            ProverType::RISC0 => {
                vec![Value::Bytes(vec![].into())]
            }
            ProverType::SP1 => {
                vec![Value::Bytes(vec![].into())]
            }
            ProverType::TDX => {
                vec![Value::Bytes(vec![].into())]
            }
            ProverType::Exec => unimplemented!("Doesn't need to generate an empty calldata."),
        }
    }

    fn verifier_getter(&self) -> Option<String> {
        // These values have to match with the OnChainProposer.sol contract
        match self {
            ProverType::RISC0 => Some("REQUIRE_RISC0_PROOF()".to_string()),
            ProverType::SP1 => Some("REQUIRE_SP1_PROOF()".to_string()),
            ProverType::TDX => Some("REQUIRE_TDX_PROOF()".to_string()),
            ProverType::Exec => None,
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
            BatchProof::ProofBytes(proof) => {
                // For TDX proofs stored as ProofBytes, the `proof` field contains
                // the signature that was previously in ProofCalldata.calldata.
                // For zkVM proofs this returns the raw proof bytes as calldata.
                if proof.proof.is_empty() {
                    vec![]
                } else {
                    vec![Value::Bytes(proof.proof.clone().into())]
                }
            }
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

/// Contains the data ready to be sent to the on-chain verifiers.
#[derive(PartialEq, Serialize, Deserialize, Clone, Debug)]
pub struct ProofCalldata {
    pub prover_type: ProverType,
    pub calldata: Vec<Value>,
}

/// Enum for the ProverServer <--> ProverClient Communication Protocol.
#[allow(clippy::large_enum_variant)]
#[derive(Serialize, Deserialize)]
pub enum ProofData {
    /// 1.
    /// The client performs any needed setup steps
    /// This includes things such as key registration
    ProverSetup {
        prover_type: ProverType,
        payload: Bytes,
    },

    /// 2.
    /// The Server acknowledges the receipt of the setup and it's completion
    ProverSetupACK,

    /// 3.
    /// The Client initiates the connection with an InputRequest.
    /// Asking for the ProverInputData the prover_server considers/needs.
    /// The commit hash is used to ensure the client and server are compatible.
    /// The prover_type tells the coordinator which backend the client runs,
    /// so it can skip batches that already have a proof for that type.
    InputRequest {
        commit_hash: String,
        prover_type: ProverType,
    },

    /// 4.
    /// The Server responds with VersionMismatch when the prover's code version
    /// does not match the version needed to prove the next batch. This can happen
    /// when the batch was stored with a different version, or when the prover is
    /// stale and future batches will use a newer version.
    VersionMismatch,

    /// 4b.
    /// The Server responds with ProverTypeNotNeeded when the connecting prover's
    /// backend type is not in the set of required proof types for this deployment.
    ProverTypeNotNeeded { prover_type: ProverType },

    /// 5.
    /// The Server responds with an InputResponse containing the ProverInputData.
    /// If the InputResponse is ProofData::InputResponse{None, None},
    /// the Client knows the InputRequest couldn't be performed.
    InputResponse {
        id: Option<u64>,
        input: Option<ProverInputData>,
        format: Option<ProofFormat>,
    },

    /// 6.
    /// The Client submits the zk Proof generated by the prover for the specified id.
    ProofSubmit { id: u64, proof: ProofBytes },

    /// 7.
    /// The Server acknowledges the receipt of the proof and updates its state,
    ProofSubmitACK { id: u64 },
}

impl ProofData {
    /// Builder function for creating a ProverSetup
    pub fn prover_setup(prover_type: ProverType, payload: Bytes) -> Self {
        ProofData::ProverSetup {
            prover_type,
            payload,
        }
    }

    /// Builder function for creating a ProverSetupACK
    pub fn prover_setup_ack() -> Self {
        ProofData::ProverSetupACK
    }

    /// Builder function for creating an InputRequest
    pub fn input_request(commit_hash: String, prover_type: ProverType) -> Self {
        ProofData::InputRequest {
            commit_hash,
            prover_type,
        }
    }

    /// Builder function for creating a VersionMismatch
    pub fn version_mismatch() -> Self {
        ProofData::VersionMismatch
    }

    /// Builder function for creating an InputResponse
    pub fn input_response(id: u64, input: ProverInputData, format: ProofFormat) -> Self {
        ProofData::InputResponse {
            id: Some(id),
            input: Some(input),
            format: Some(format),
        }
    }

    pub fn empty_input_response() -> Self {
        ProofData::InputResponse {
            id: None,
            input: None,
            format: None,
        }
    }

    /// Builder function for creating a ProofSubmit
    pub fn proof_submit(id: u64, proof: ProofBytes) -> Self {
        ProofData::ProofSubmit { id, proof }
    }

    /// Builder function for creating a ProofSubmitAck
    pub fn proof_submit_ack(id: u64) -> Self {
        ProofData::ProofSubmitACK { id }
    }
}
