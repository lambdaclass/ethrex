use ethrex_common::types::prover::{ProofBytes, ProofFormat, ProverOutput, ProverType};
use ethrex_guest_program::{ZKVM_SP1_PROGRAM_ELF, input::ProgramInput};
use rkyv::rancor::Error;
#[cfg(not(feature = "gpu"))]
use sp1_sdk::blocking::{CpuProver as Sp1Prover, CpuProverBuilder};
#[cfg(feature = "gpu")]
use sp1_sdk::blocking::{CudaProver as Sp1Prover, CudaProverBuilder};
use sp1_sdk::{
    Elf, HashableKey, ProvingKey, SP1ProofMode, SP1ProofWithPublicValues, SP1Stdin,
    SP1VerifyingKey,
    blocking::{ProveRequest, Prover},
};
use std::{
    fmt::Debug,
    sync::OnceLock,
    time::{Duration, Instant},
};
use tracing::warn;
use url::Url;

use crate::backend::{BackendError, ProverBackend};

/// Setup data for the SP1 prover (client, proving key, verifying key).
pub struct ProverSetup {
    client: Sp1Prover,
    pk: <Sp1Prover as Prover>::ProvingKey,
    vk: SP1VerifyingKey,
}

/// Global prover setup - initialized once and reused.
pub static PROVER_SETUP: OnceLock<ProverSetup> = OnceLock::new();

/// The guest program, in the form the SP1 SDK takes it.
fn guest_elf() -> Elf {
    Elf::Static(ZKVM_SP1_PROGRAM_ELF)
}

/// Runs `f` on a new thread, outside any tokio runtime. SP1's blocking API drives its
/// own runtime with `block_on`, which panics on a thread that is already running one,
/// and the prover calls this backend from async code.
fn outside_runtime<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| match scope.spawn(f).join() {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    })
}

pub fn init_prover_setup(endpoint: Option<Url>) -> ProverSetup {
    // SP1 6 has no remote prover server: CUDA proving runs on a local
    // `sp1-gpu-server` that the SDK downloads and starts, so an endpoint can only be ignored.
    if let Some(endpoint) = endpoint {
        warn!(%endpoint, "Ignoring the SP1 server URL: SP1 6 has no remote prover server");
    }
    outside_runtime(|| {
        #[cfg(feature = "gpu")]
        let client = CudaProverBuilder::new().build();
        #[cfg(not(feature = "gpu"))]
        let client = CpuProverBuilder::new().build();
        #[expect(clippy::expect_used)]
        let pk = client
            .setup(guest_elf())
            .expect("SP1 setup of the guest program failed");
        let vk = ProvingKey::verifying_key(&pk).clone();

        ProverSetup { client, pk, vk }
    })
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

    fn to_groth16_proof_bytes(proof: &Sp1ProveOutput) -> ProverOutput {
        ProverOutput::Proof(ProofBytes {
            prover_type: ProverType::SP1,
            proof: proof.proof.bytes(),
        })
    }

    /// Execute using already-serialized input.
    fn execute_with_stdin(&self, stdin: SP1Stdin) -> Result<(), BackendError> {
        let setup = self.get_setup();
        outside_runtime(|| setup.client.execute(guest_elf(), stdin).run())
            .map_err(BackendError::execution)?;
        Ok(())
    }

    /// Prove using already-serialized input.
    fn prove_with_stdin(
        &self,
        stdin: SP1Stdin,
        format: ProofFormat,
    ) -> Result<Sp1ProveOutput, BackendError> {
        let setup = self.get_setup();
        let sp1_format = Self::convert_format(format);
        let proof = outside_runtime(|| setup.client.prove(&setup.pk, stdin).mode(sp1_format).run())
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
        // On the L1 path `ProgramInput` IS the spec's `statelessInputBytes`, so it
        // must reach the guest byte-for-byte — rkyv-wrapping it would make the
        // guest's schema-prefix check fail. Only the L2 batch input is rkyv.
        #[cfg(feature = "l2")]
        let bytes = rkyv::to_bytes::<Error>(input).map_err(BackendError::serialization)?;
        #[cfg(not(feature = "l2"))]
        let bytes = input;

        let mut stdin = SP1Stdin::new();
        stdin.write_slice(bytes.as_slice());
        Ok(stdin)
    }

    fn execute(&self, input: ProgramInput) -> Result<(), BackendError> {
        let stdin = self.serialize_input(&input)?;
        self.execute_with_stdin(stdin)
    }

    fn prove(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<Self::ProofOutput, BackendError> {
        let stdin = self.serialize_input(&input)?;
        self.prove_with_stdin(stdin, format)
    }

    fn verify(&self, proof: &Self::ProofOutput) -> Result<(), BackendError> {
        let setup = self.get_setup();
        setup
            .client
            .verify(&proof.proof, &proof.vk, None)
            .map_err(BackendError::verification)?;

        Ok(())
    }

    fn to_proof_bytes(
        &self,
        proof: Self::ProofOutput,
        format: ProofFormat,
    ) -> Result<ProverOutput, BackendError> {
        let prover_output = match format {
            ProofFormat::Compressed => ProverOutput::ProofWithPublicValues {
                proof_bytes: ProofBytes {
                    prover_type: ProverType::SP1,
                    proof: bincode::serialize(&proof.proof)
                        .map_err(BackendError::proof_conversion)?,
                },
                public_values: proof.proof.public_values.to_vec(),
            },
            ProofFormat::Groth16 => Self::to_groth16_proof_bytes(&proof),
        };

        Ok(prover_output)
    }

    fn execute_timed(&self, input: ProgramInput) -> Result<Duration, BackendError> {
        let stdin = self.serialize_input(&input)?;
        let start = Instant::now();
        self.execute_with_stdin(stdin)?;
        Ok(start.elapsed())
    }

    fn prove_timed(
        &self,
        input: ProgramInput,
        format: ProofFormat,
    ) -> Result<(Self::ProofOutput, Duration), BackendError> {
        let stdin = self.serialize_input(&input)?;
        let start = Instant::now();
        let proof = self.prove_with_stdin(stdin, format)?;
        Ok((proof, start.elapsed()))
    }
}
