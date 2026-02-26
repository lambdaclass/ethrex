use ethrex_guest_program::{ZKVM_SP1_PROGRAM_ELF, input::ProgramInput};
use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofBytes, ProofCalldata, ProofFormat, ProverType},
};
use rkyv::rancor::Error;
use sp1_prover::components::CpuProverComponents;
#[cfg(not(feature = "gpu"))]
use sp1_sdk::CpuProver;
#[cfg(feature = "gpu")]
use sp1_sdk::cuda::builder::CudaProverBuilder;
use sp1_sdk::{
    HashableKey, Prover, SP1ProofMode, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin,
    SP1VerifyingKey,
};
use std::{
    fmt::Debug,
    sync::OnceLock,
    time::{Duration, Instant},
};
use url::Url;

use crate::backend::{BackendError, ProverBackend};

/// Setup data for the SP1 prover (client, proving key, verifying key).
pub struct ProverSetup {
    client: Box<dyn Prover<CpuProverComponents>>,
    pk: SP1ProvingKey,
    vk: SP1VerifyingKey,
}

/// Global prover setup - initialized once and reused.
pub static PROVER_SETUP: OnceLock<ProverSetup> = OnceLock::new();

pub fn init_prover_setup(_endpoint: Option<Url>) -> ProverSetup {
    #[cfg(feature = "gpu")]
    let client = {
        if let Some(endpoint) = _endpoint {
            CudaProverBuilder::default()
                .server(
                    #[expect(clippy::expect_used)]
                    endpoint
                        .join("/twirp/")
                        .expect("Failed to parse moongate server url")
                        .as_ref(),
                )
                .build()
        } else {
            CudaProverBuilder::default().local().build()
        }
    };
    #[cfg(not(feature = "gpu"))]
    let client = { CpuProver::new() };
    let (pk, vk) = client.setup(ZKVM_SP1_PROGRAM_ELF);

    ProverSetup {
        client: Box::new(client),
        pk,
        vk,
    }
}

/// SP1-specific proof output containing the proof and verifying key.
pub struct Sp1ProveOutput {
    pub proof: SP1ProofWithPublicValues,
    pub vk: SP1VerifyingKey,
}

impl Debug for Sp1ProveOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sp1ProveOutput")
            .field("proof", &self.proof)
            .field("vk", &self.vk.bytes32())
            .finish()
    }
}

impl Sp1ProveOutput {
    pub fn new(proof: SP1ProofWithPublicValues, verifying_key: SP1VerifyingKey) -> Self {
        Sp1ProveOutput {
            proof,
            vk: verifying_key,
        }
    }
}

/// SP1 prover backend.
#[derive(Default)]
pub struct Sp1Backend;

impl Sp1Backend {
    pub fn new() -> Self {
        Self
    }

    fn get_setup(&self) -> &ProverSetup {
        PROVER_SETUP.get_or_init(|| init_prover_setup(None))
    }

    fn convert_format(format: ProofFormat) -> SP1ProofMode {
        match format {
            ProofFormat::Compressed => SP1ProofMode::Compressed,
            ProofFormat::Groth16 => SP1ProofMode::Groth16,
        }
    }

    fn to_calldata(proof: &Sp1ProveOutput) -> ProofCalldata {
        let calldata = vec![Value::Bytes(proof.proof.bytes().into())];

        ProofCalldata {
            prover_type: ProverType::SP1,
            calldata,
        }
    }

    /// Execute using already-serialized input.
    fn execute_with_stdin(&self, stdin: &SP1Stdin) -> Result<(), BackendError> {
        let setup = self.get_setup();
        setup
            .client
            .execute(ZKVM_SP1_PROGRAM_ELF, stdin)
            .map_err(BackendError::execution)?;
        Ok(())
    }

    /// Prove using already-serialized input.
    fn prove_with_stdin(
        &self,
        stdin: &SP1Stdin,
        format: ProofFormat,
    ) -> Result<Sp1ProveOutput, BackendError> {
        let setup = self.get_setup();
        let sp1_format = Self::convert_format(format);
        let proof = setup
            .client
            .prove(&setup.pk, stdin, sp1_format)
            .map_err(BackendError::proving)?;
        Ok(Sp1ProveOutput::new(proof, setup.vk.clone()))
    }
}

impl ProverBackend for Sp1Backend {
    type ProofOutput = Sp1ProveOutput;
    type SerializedInput = SP1Stdin;

    fn prover_type(&self) -> ProverType {
        ProverType::SP1
    }

    fn serialize_input(&self, input: &ProgramInput) -> Result<Self::SerializedInput, BackendError> {
        let mut stdin = SP1Stdin::new();
        let bytes = rkyv::to_bytes::<Error>(input).map_err(BackendError::serialization)?;
        stdin.write_slice(bytes.as_slice());
        Ok(stdin)
    }

    fn execute(&self, input: ProgramInput) -> Result<(), BackendError> {
        let stdin = self.serialize_input(&input)?;
        self.execute_with_stdin(&stdin)
    }

    fn prove(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        let stdin = self.serialize_input(&input)?;
        self.prove_with_stdin(&stdin, format)
    }

    fn verify(&self, proof: &Self::ProofOutput) -> Result<(), BackendError> {
        let setup = self.get_setup();
        setup
            .client
            .verify(&proof.proof, &proof.vk)
            .map_err(BackendError::verification)?;

        Ok(())
    }

    fn to_batch_proof(
        &self,
        proof: Self::ProofOutput,
        format: ProofFormat,
    ) -> Result<BatchProof, BackendError> {
        let batch_proof = match format {
            ProofFormat::Compressed => BatchProof::ProofBytes(ProofBytes {
                prover_type: ProverType::SP1,
                proof: bincode::serialize(&proof.proof).map_err(BackendError::batch_proof)?,
                public_values: proof.proof.public_values.to_vec(),
            }),
            ProofFormat::Groth16 => BatchProof::ProofCalldata(Self::to_calldata(&proof)),
        };

        Ok(batch_proof)
    }

    fn execute_timed(&self, input: ProgramInput) -> Result<Duration, BackendError> {
        let stdin = self.serialize_input(&input)?;
        let start = Instant::now();
        self.execute_with_stdin(&stdin)?;
        Ok(start.elapsed())
    }

    fn prove_timed(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<(Self::ProofOutput, Duration), BackendError> {
        let stdin = self.serialize_input(&input)?;
        let start = Instant::now();
        let proof = self.prove_with_stdin(&stdin, format)?;
        Ok((proof, start.elapsed()))
    }
}
