use std::{fmt::Debug, sync::LazyLock};

use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofBytes, ProofCalldata, ProverType},
};
use guest_program::input::ProgramInput;
use rkyv::rancor::Error;
use sp1_sdk::{
    EnvProver, HashableKey, ProverClient, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin,
    SP1VerifyingKey,
};
use std::time::Instant;
use tracing::info;

#[cfg(not(clippy))]
static PROGRAM_ELF: &[u8] =
    include_bytes!("../guest_program/src/sp1/out/riscv32im-succinct-zkvm-elf");

// If we're running clippy, the file isn't generated.
// To avoid compilation errors, we override it with an empty slice.
#[cfg(clippy)]
static PROGRAM_ELF: &[u8] = &[];

struct ProverSetup {
    client: EnvProver,
    pk: SP1ProvingKey,
    vk: SP1VerifyingKey,
}

static PROVER_SETUP: LazyLock<ProverSetup> = LazyLock::new(|| {
    let client = ProverClient::from_env();
    let (pk, vk) = client.setup(PROGRAM_ELF);
    ProverSetup { client, pk, vk }
});

pub struct ProveOutput {
    pub proof: SP1ProofWithPublicValues,
    pub vk: SP1VerifyingKey,
}

// TODO: Error enum

impl Debug for ProveOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sp1Proof")
            .field("proof", &self.proof)
            .field("vk", &self.vk.bytes32())
            .finish()
    }
}

impl ProveOutput {
    pub fn new(proof: SP1ProofWithPublicValues, verifying_key: SP1VerifyingKey) -> Self {
        ProveOutput {
            proof,
            vk: verifying_key,
        }
    }
}

/// Execute a program using the SP1 backend with panic handling.
/// 
/// This function executes the zkVM program and catches any panics that may occur
/// during execution, converting them to proper error results.
pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), Box<dyn std::error::Error>> {
        let mut stdin = SP1Stdin::new();
        let bytes = rkyv::to_bytes::<Error>(&input)?;
        stdin.write_slice(bytes.as_slice());

        let setup = &*PROVER_SETUP;

        let now = Instant::now();
        setup.client.execute(PROGRAM_ELF, &stdin).run()?;
        let elapsed = now.elapsed();

        info!("Successfully executed SP1 program in {:.2?}", elapsed);
        Ok(())
    }));
    
    match result {
        Ok(exec_result) => exec_result,
        Err(panic_info) => {
            let panic_msg = extract_panic_message(&panic_info);
            
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("SP1 execution panicked: {}. This may be due to insufficient memory or invalid program input.", panic_msg)
            )))
        }
    }
}

/// Generate a proof using the SP1 backend with panic handling.
/// 
/// This function generates a zkVM proof and catches any panics that may occur
/// during proving, converting them to proper error results.
pub fn prove(
    input: ProgramInput,
    aligned_mode: bool,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<ProveOutput, Box<dyn std::error::Error>> {
        let mut stdin = SP1Stdin::new();
        let bytes = rkyv::to_bytes::<Error>(&input)?;
        stdin.write_slice(bytes.as_slice());

        let setup = &*PROVER_SETUP;

        // Generate proof based on the requested mode
        let proof = if aligned_mode {
            setup.client.prove(&setup.pk, &stdin).compressed().run()?
        } else {
            setup.client.prove(&setup.pk, &stdin).groth16().run()?
        };

        info!("Successfully generated SP1Proof.");
        Ok(ProveOutput::new(proof, setup.vk.clone()))
    }));
    
    match result {
        Ok(prove_result) => prove_result,
        Err(panic_info) => {
            let panic_msg = extract_panic_message(&panic_info);
            
            let mode_str = if aligned_mode { "compressed" } else { "groth16" };
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("SP1 proving ({} mode) panicked: {}. This may be due to insufficient memory, invalid program input, or zkVM constraints violation.", mode_str, panic_msg)
            )))
        }
    }
}

pub fn verify(output: &ProveOutput) -> Result<(), Box<dyn std::error::Error>> {
    let setup = &*PROVER_SETUP;
    setup.client.verify(&output.proof, &output.vk)?;

    Ok(())
}

pub fn to_batch_proof(
    proof: ProveOutput,
    aligned_mode: bool,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    let batch_proof = if aligned_mode {
        BatchProof::ProofBytes(ProofBytes {
            proof: bincode::serialize(&proof.proof)?,
            public_values: proof.proof.public_values.to_vec(),
        })
    } else {
        BatchProof::ProofCalldata(to_calldata(proof))
    };

    Ok(batch_proof)
}

fn to_calldata(proof: ProveOutput) -> ProofCalldata {
    // bytes calldata publicValues,
    // bytes calldata proofBytes
    let calldata = vec![
        Value::Bytes(proof.proof.public_values.to_vec().into()),
        Value::Bytes(proof.proof.bytes().into()),
    ];

    ProofCalldata {
        prover_type: ProverType::SP1,
        calldata,
    }
}

/// Extract a meaningful error message from panic information.
fn extract_panic_message(panic_info: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic_info.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = panic_info.downcast_ref::<&str>() {
        s.to_string()
    } else {
        "Unknown panic occurred".to_string()
    }
}
