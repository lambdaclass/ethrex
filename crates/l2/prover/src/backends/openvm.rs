pub struct ProgramOutput(pub [u8; 32]);

pub fn execute(_input: zkvm_interface::io::ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    unimplemented!("OpenVM execute is not implemented yet");
}

pub fn prove(
    _input: zkvm_interface::io::ProgramInput,
    _aligned_mode: bool,
) -> Result<ProgramOutput, Box<dyn std::error::Error>> {
    unimplemented!("OpenVM prove is not implemented yet");
}

pub fn to_batch_proof(
    _aligned_mode: bool,
) -> Result<ethrex_l2_common::prover::BatchProof, Box<dyn std::error::Error>> {
    unimplemented!("OpenVM to_batch_proof is not implemented yet");
}
