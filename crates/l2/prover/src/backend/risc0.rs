use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofBytes, ProofCalldata, ProofFormat, ProverType},
};
use guest_program::{
    input::{JSONProgramInput, ProgramInput},
    methods::{ZKVM_RISC0_PROGRAM_ELF, ZKVM_RISC0_PROGRAM_ID},
};
use risc0_zkp::verify::VerificationError;
use risc0_zkvm::{
    ExecutorEnv, InnerReceipt, ProverOpts, Receipt, default_executor, default_prover,
    serde::Error as Risc0SerdeError,
};
use std::time::Instant;
use tracing::info;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("can only encode groth16 seals")]
    EncodeNonGroth16Seal,
    #[error("failed to get seal selector")]
    NoSealSelector,
    #[error("verification failed: {0}")]
    VerificationFailed(#[from] VerificationError),
    #[error("decode failed: {0}")]
    Risc0SerdeError(#[from] Risc0SerdeError),
    #[error("zkvm dynamic error: {0}")]
    ZkvmDyn(#[from] anyhow::Error),
    #[error("bincode error: {0}")]
    Bincode(#[from] Box<bincode::ErrorKind>),
}

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    let env = ExecutorEnv::builder()
        .write(&JSONProgramInput(input))?
        .build()?;

    let executor = default_executor();

    let now = Instant::now();
    let _session_info = executor.execute(env, ZKVM_RISC0_PROGRAM_ELF)?;
    let elapsed = now.elapsed();

    info!("Successfully generated session info in {:.2?}", elapsed);
    Ok(())
}

pub fn prove(
    input: ProgramInput,
    format: ProofFormat,
) -> Result<Receipt, Box<dyn std::error::Error>> {
    let mut stdout = Vec::new();

    let env = ExecutorEnv::builder()
        .stdout(&mut stdout)
        .write(&JSONProgramInput(input))?
        .build()?;

    let prover = default_prover();

    let prover_opts = match format {
        ProofFormat::Compressed => ProverOpts::succinct(),
        ProofFormat::Groth16 => ProverOpts::groth16(),
    };
    let prove_info = prover.prove_with_opts(env, ZKVM_RISC0_PROGRAM_ELF, &prover_opts)?;

    info!("Successfully generated execution receipt.");
    Ok(prove_info.receipt)
}

pub fn verify(receipt: &Receipt) -> Result<(), Error> {
    receipt.verify(ZKVM_RISC0_PROGRAM_ID)?;
    Ok(())
}

pub fn to_batch_proof(
    receipt: Receipt,
    format: ProofFormat,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    let batch_proof = match format {
        ProofFormat::Compressed => BatchProof::ProofBytes(ProofBytes {
            prover_type: ProverType::RISC0,
            proof: bincode::serialize(&receipt.inner)?,
            public_values: receipt.journal.bytes,
        }),
        ProofFormat::Groth16 => BatchProof::ProofCalldata(to_calldata(receipt)?),
    };

    Ok(batch_proof)
}

fn to_calldata(receipt: Receipt) -> Result<ProofCalldata, Error> {
    let seal = encode_seal(&receipt)?;
    let journal = receipt.journal.bytes;

    // bytes calldata seal,
    // bytes32 imageId,
    // bytes journal
    let calldata = vec![Value::Bytes(seal.into()), Value::Bytes(journal.into())];

    Ok(ProofCalldata {
        prover_type: ProverType::RISC0,
        calldata,
    })
}

// ref: https://github.com/risc0/risc0-ethereum/blob/046bb34ea4605f9d8420c7db89baf8e1064fa6f5/contracts/src/lib.rs#L88
// this was reimplemented because risc0-ethereum-contracts brings a different version of c-kzg into the workspace (2.1.0),
// which is incompatible with our current version (1.0.3).
fn encode_seal(receipt: &Receipt) -> Result<Vec<u8>, Error> {
    let InnerReceipt::Groth16(receipt) = receipt.inner.clone() else {
        return Err(Error::EncodeNonGroth16Seal);
    };
    let selector = &receipt
        .verifier_parameters
        .as_bytes()
        .get(..4)
        .ok_or(Error::NoSealSelector)?;
    // Create a new vector with the capacity to hold both selector and seal
    let mut selector_seal = Vec::with_capacity(selector.len() + receipt.seal.len());
    selector_seal.extend_from_slice(selector);
    selector_seal.extend_from_slice(receipt.seal.as_ref());
    Ok(selector_seal)
}
