use ethrex_l2::utils::prover::proving_systems::{ProofCalldata, ProverType};
use ethrex_l2_sdk::calldata::Value;
use risc0_ethereum_contracts::encode_seal;
use risc0_zkvm::{
    default_executor, default_prover, sha::Digestible, ExecutorEnv, ProverOpts, Receipt,
};
use tracing::info;
use zkvm_interface::{
    io::ProgramInput,
    methods::{ZKVM_RISC0_PROGRAM_ELF, ZKVM_RISC0_PROGRAM_ID},
};

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    let env = ExecutorEnv::builder().write(&input)?.build()?;

    let executor = default_executor();

    let session_info = executor.execute(env, ZKVM_RISC0_PROGRAM_ELF)?;

    info!("Successfully generated session info.");
    Ok(())
}

pub fn prove(input: ProgramInput) -> Result<Receipt, Box<dyn std::error::Error>> {
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

pub fn to_calldata(receipt: Receipt) -> Result<ProofCalldata, Box<dyn std::error::Error>> {
    let seal = encode_seal(&receipt)?;
    let image_id = ZKVM_RISC0_PROGRAM_ID;
    let journal = receipt.journal.bytes;

    // convert image_id into bytes
    let image_id = {
        let mut res = [0; 32];
        for i in 0..8 {
            res[4 * i..][..4].copy_from_slice(&image_id[i].to_be_bytes());
        }
        res.to_vec()
    };

    // bytes calldata seal,
    // bytes32 imageId,
    // bytes journal
    let calldata = vec![
        Value::Bytes(seal.into()),
        Value::FixedBytes(image_id.into()),
        Value::Bytes(journal.into()),
    ];

    Ok(ProofCalldata {
        prover_type: ProverType::RISC0,
        calldata,
    })
}
