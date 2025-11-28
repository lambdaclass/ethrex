use ethrex_l2_common::prover::{BatchProof, ProofCalldata, ProofFormat};
use guest_program::{ZKVM_PICO_PROGRAM_ELF, input::ProgramInput};
use pico_sdk::client::DefaultProverClient;
use pico_vm::{
    instances::configs::embed_kb_bn254_poseidon2::KoalaBearBn254Poseidon2,
    machine::proof::MetaProof,
};
use rkyv::rancor::Error;
use tracing::warn;

pub type ProveOutput = MetaProof<KoalaBearBn254Poseidon2>;

pub fn prove(
    input: ProgramInput,
    _format: ProofFormat,
) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    if cfg!(feature = "gpu") {
        warn!("The Pico backend doesn't support GPU proving, falling back to CPU.");
    }

    let client = DefaultProverClient::new(ZKVM_PICO_PROGRAM_ELF);

    let mut stdin = client.new_stdin_builder();
    let input_bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_slice(&input_bytes);

    let (_, compressed_proof) = client.prove(stdin)?;
    Ok(compressed_proof)
}

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    let client = DefaultProverClient::new(ZKVM_PICO_PROGRAM_ELF);

    let mut stdin = client.new_stdin_builder();
    let input_bytes = rkyv::to_bytes::<Error>(&input)?;
    stdin.write_slice(&input_bytes);

    client.emulate(stdin);
    Ok(())
}

pub fn verify(_output: &ProveOutput) -> Result<(), Box<dyn std::error::Error>> {
    warn!(
        "Pico backend's verify() does nothing, this is because Pico doesn't expose a verification function but will verify each phase during proving as a sanity check"
    );
    Ok(())
}

pub fn to_batch_proof(
    proof: ProveOutput,
    _format: ProofFormat,
) -> Result<BatchProof, Box<dyn std::error::Error>> {
    Ok(BatchProof::ProofCalldata(to_calldata(proof)))
}

fn to_calldata(output: ProveOutput) -> ProofCalldata {
    unimplemented!();
    // let ProveOutput {
    //     public_values,
    //     proof,
    // } = output;

    // // TODO: double check big endian is correct
    // let proof = proof
    //     .chunks(32)
    //     .map(|integer| Value::Int(U256::from_big_endian(integer)))
    //     .collect();

    // // bytes calldata publicValues,
    // // uint256[8] calldata proof
    // let calldata = vec![Value::Bytes(public_values.into()), Value::FixedArray(proof)];

    // ProofCalldata {
    //     prover_type: ProverType::Pico,
    //     calldata,
    // }
}
