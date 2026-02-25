use ethrex_guest_program::{ZKVM_SP1_PROGRAM_ELF, input::ProgramInput};
use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofBytes, ProofCalldata, ProofFormat, ProverType},
};
use rkyv::rancor::Error;
#[cfg(not(feature = "gpu"))]
use sp1_sdk::blocking::CpuProver;
use sp1_sdk::{
    Elf, HashableKey, ProvingKey as _, SP1ProofMode, SP1ProofWithPublicValues, SP1Stdin,
    SP1VerifyingKey,
    blocking::{ProveRequest as _, Prover},
};
use std::{
    fmt::Debug,
    sync::OnceLock,
    time::{Duration, Instant},
};
use url::Url;

use crate::backend::{BackendError, ProverBackend};

#[cfg(not(feature = "gpu"))]
type ConcreteProver = CpuProver;
#[cfg(feature = "gpu")]
type ConcreteProver = sp1_sdk::blocking::CudaProver;

type ConcreteProvingKey = <ConcreteProver as Prover>::ProvingKey;

/// Setup data for the SP1 prover (client, proving key, verifying key).
pub struct ProverSetup {
    client: ConcreteProver,
    pk: ConcreteProvingKey,
    vk: SP1VerifyingKey,
}

/// Global prover setup - initialized once and reused.
pub static PROVER_SETUP: OnceLock<ProverSetup> = OnceLock::new();

/// Initialize the SP1 prover client, proving key, and verifying key.
///
/// **Important:** This function must NOT be called from within a tokio runtime when the
/// `gpu` feature is enabled. The `CudaProver` builder internally calls `block_on()`, which
/// panics if a tokio runtime is already active on the current thread. Use [`Sp1Backend::get_setup`]
/// instead, which handles this by spawning initialization on a separate OS thread.
///
/// `CpuProver::new()` does not have this limitation and can be called from any context.
pub fn init_prover_setup(_endpoint: Option<Url>) -> Result<ProverSetup, String> {
    #[cfg(not(feature = "gpu"))]
    let client = CpuProver::new();
    #[cfg(feature = "gpu")]
    let client = sp1_sdk::blocking::ProverClient::builder().cuda().build();

    let elf = Elf::from(ZKVM_SP1_PROGRAM_ELF);
    let pk = client.setup(elf).map_err(|e| format!("Failed to setup SP1 prover: {e}"))?;
    let vk = pk.verifying_key().clone();

    Ok(ProverSetup { client, pk, vk })
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

    /// Returns the global prover setup, initializing it on first call.
    ///
    /// Initialization is spawned on a separate OS thread because this method is called from
    /// within a tokio runtime (the prover's async loop), and the SP1 `CudaProver` builder
    /// internally calls `block_on()`. Calling `block_on()` from within an active tokio
    /// runtime panics with "Cannot start a runtime from within a runtime". Spawning on a
    /// fresh thread avoids this since that thread has no associated runtime.
    ///
    /// This only runs once thanks to `OnceLock` — subsequent calls return the cached setup.
    fn get_setup(&self) -> Result<&ProverSetup, BackendError> {
        if let Some(setup) = PROVER_SETUP.get() {
            return Ok(setup);
        }
        let setup = std::thread::spawn(|| init_prover_setup(None))
            .join()
            .map_err(|e| BackendError::initialization(format!("SP1 setup thread panicked: {e:?}")))?
            .map_err(BackendError::initialization)?;
        Ok(PROVER_SETUP.get_or_init(|| setup))
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
    ///
    /// Runs on a scoped thread because the SP1 blocking SDK uses `block_on()` internally,
    /// which panics inside a tokio runtime. `std::thread::scope` lets us borrow `setup`
    /// safely across the thread boundary.
    fn execute_with_stdin(&self, stdin: &SP1Stdin) -> Result<(), BackendError> {
        let setup = self.get_setup()?;
        let elf = Elf::from(ZKVM_SP1_PROGRAM_ELF);
        let stdin = stdin.clone();
        std::thread::scope(|s| {
            s.spawn(|| {
                setup
                    .client
                    .execute(elf, stdin)
                    .run()
                    .map_err(BackendError::execution)
            })
            .join()
            .map_err(|e| BackendError::execution(format!("SP1 execute thread panicked: {e:?}")))?
        })?;
        Ok(())
    }

    /// Prove using already-serialized input.
    ///
    /// Runs on a scoped thread because the SP1 blocking SDK uses `block_on()` internally,
    /// which panics inside a tokio runtime.
    fn prove_with_stdin(
        &self,
        stdin: &SP1Stdin,
        format: ProofFormat,
    ) -> Result<Sp1ProveOutput, BackendError> {
        let setup = self.get_setup()?;
        let sp1_format = Self::convert_format(format);
        let stdin = stdin.clone();
        let proof = std::thread::scope(|s| {
            s.spawn(|| {
                setup
                    .client
                    .prove(&setup.pk, stdin)
                    .mode(sp1_format)
                    .run()
                    .map_err(BackendError::proving)
            })
            .join()
            .map_err(|e| BackendError::proving(format!("SP1 prove thread panicked: {e:?}")))?
        })?;
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
        let setup = self.get_setup()?;
        std::thread::scope(|s| {
            s.spawn(|| {
                setup
                    .client
                    .verify(&proof.proof, &proof.vk, None)
                    .map_err(BackendError::verification)
            })
            .join()
            .map_err(|e| BackendError::verification(format!("SP1 verify thread panicked: {e:?}")))?
        })?;
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
