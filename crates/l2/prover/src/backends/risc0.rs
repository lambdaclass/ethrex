use ethrex_l2_sdk::calldata::Value;
use risc0_ethereum_contracts::encode_seal;
use risc0_zkvm::{default_prover, sha::Digestible, ExecutorEnv, ProverOpts, Receipt};
use tracing::info;
use zkvm_interface::io::ProgramInput;

include!(concat!(env!("OUT_DIR"), "/methods.rs"));

fn execute(&mut self, input: ProgramInput) -> Result<ExecuteOutput, Box<dyn std::error::Error>> {
    unimplemented!()
}

fn prove(input: ProgramInput) -> Result<Receipt, Box<dyn std::error::Error>> {
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

pub fn verify(receipt: &Receipt) -> Result<(), Box<dyn std::error::Error>> {
    receipt.verify(ZKVM_RISC0_PROGRAM_ID)?;
    Ok(())
}

pub fn to_calldata(receipt: Receipt) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let seal = encode_seal(&receipt)?;
    let image_id = ZKVM_RISC_PROGRAM_ID;
    let journal_digest = receipt.journal.digest().as_bytes();

    // bytes calldata seal,
    // bytes32 imageId,
    // bytes32 journalDigest
    Ok(vec![
        Value::Bytes(seal.into()),
        Value::FixedBytes(image_id.into()),
        Value::FixedBytes(journal_digest.into()),
    ])
}

fn get_gas(stdout: &[u8]) -> Result<u64, Box<dyn std::error::Error>> {
    unimplemented!()
    // TODO: return stdout as proving output
    // Ok(risc0_zkvm::serde::from_slice(
    //     stdout.get(..8).unwrap_or_default(), // first 8 bytes
    // )?)
}
