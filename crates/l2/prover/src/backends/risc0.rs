use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofCalldata, ProverType},
};
use risc0_zkp::verify::VerificationError;
use risc0_zkvm::{
    ExecutorEnv, InnerReceipt, ProverOpts, default_executor, default_prover,
    serde::Error as Risc0SerdeError,
};
use tracing::info;
use zkvm_interface::{
    io::ProgramInput,
    methods::{ZKVM_RISC0_PROGRAM_ELF, ZKVM_RISC0_PROGRAM_ID},
};

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
}

pub fn execute(input: ProgramInput) -> Result<(), Error> {
    let env = ExecutorEnv::builder().write(&input)?.build()?;

    let executor = default_executor();

    let _session_info = executor.execute(env, ZKVM_RISC0_PROGRAM_ELF)?;

    info!("Successfully generated session info.");
    Ok(())
}

pub fn prove(input: ProgramInput, aligned_mode: bool) -> Result<risc0_zkvm::Receipt, Error> {
    let mut stdout = Vec::new();

    let env = ExecutorEnv::builder()
        .stdout(&mut stdout)
        .write(&input)?
        .build()?;

    let prover = default_prover();

    // contains the receipt along with statistics about execution of the guest
    let prove_info = prover.prove_with_opts(env, ZKVM_RISC0_PROGRAM_ELF, &ProverOpts::groth16())?;

    info!("Successfully generated execution receipt.");
    Ok(prove_info.receipt)
}

pub fn verify(receipt: &risc0_zkvm::Receipt) -> Result<(), Error> {
    receipt.verify(ZKVM_RISC0_PROGRAM_ID)?;
    Ok(())
}

pub fn to_batch_proof(proof: risc0_zkvm::Receipt, aligned_mode: bool) -> Result<BatchProof, Error> {
    let batch_proof = if aligned_mode {
        BatchProof::ProofBytes(ProofBytes {
            proof: bincode::serialize(&proof.proof)?,
            public_values: proof.proof.public_values.to_vec(),
        })
    } else {
        BatchProof::ProofCalldata(to_calldata(proof))
    };

    to_calldata(proof).map(BatchProof::ProofCalldata)
}

fn to_calldata(receipt: risc0_zkvm::Receipt) -> Result<ProofCalldata, Error> {
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
fn encode_seal(receipt: &risc0_zkvm::Receipt) -> Result<Vec<u8>, Error> {
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
