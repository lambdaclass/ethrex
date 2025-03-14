use ethrex_blockchain::{validate_block, validate_gas_used};
use ethrex_l2::utils::prover::proving_systems::{ProofCalldata, ProverType};
use ethrex_l2_sdk::calldata::Value;
use ethrex_vm::backends::revm::{db::EvmState, REVM};
use tracing::warn;
use zkvm_interface::{
    io::{ProgramInput, ProgramOutput},
    trie::{update_tries, verify_db},
};

pub struct ProveOutput(pub ProgramOutput);

pub fn execute(input: ProgramInput) -> Result<(), Box<dyn std::error::Error>> {
    execution_program(input)?;
    Ok(())
}

pub fn prove(input: ProgramInput) -> Result<ProveOutput, Box<dyn std::error::Error>> {
    warn!("\"exec\" prover backend generates no proof, only executes");
    let output = execution_program(input)?;
    Ok(ProveOutput(output))
}

pub fn verify(_proof: &ProveOutput) -> Result<(), Box<dyn std::error::Error>> {
    warn!("\"exec\" prover backend generates no proof, verification always succeeds");
    Ok(())
}

pub fn to_calldata(proof: ProveOutput) -> Result<ProofCalldata, Box<dyn std::error::Error>> {
    let public_inputs = proof.0.encode();
    Ok(ProofCalldata {
        prover_type: ProverType::Exec,
        calldata: vec![Value::Bytes(public_inputs.into())],
    })
}

fn execution_program(input: ProgramInput) -> Result<ProgramOutput, Box<dyn std::error::Error>> {
    let ProgramInput {
        block,
        parent_block_header,
        db,
    } = input;
    let mut state = EvmState::from(db.clone());
    let chain_config = state
        .chain_config()?;

    // Validate the block
    validate_block(&block, &parent_block_header, &chain_config)?;

    // Tries used for validating initial and final state root
    let (mut state_trie, mut storage_tries) = db
        .get_tries()?;

    // Validate the initial state
    let initial_state_hash = state_trie.hash_no_commit();
    if initial_state_hash != parent_block_header.state_root {
        return Err("invalid initial state trie".to_string().into())
    }
    if !verify_db(&db, &state_trie, &storage_tries)? {
        return Err("invalid database".to_string().into())
    };

    let result = REVM::execute_block(&block, &mut state)?;
    let receipts = result.receipts;
    let account_updates = result.account_updates;
    validate_gas_used(&receipts, &block.header)?;

    // Update state trie
    update_tries(&mut state_trie, &mut storage_tries, &account_updates)?;

    // Calculate final state root hash and check
    let final_state_hash = state_trie.hash_no_commit();
    if final_state_hash != block.header.state_root {
        return Err("invalid final state trie".to_string().into());
    }

    Ok(ProgramOutput {
        initial_state_hash,
        final_state_hash,
    })
}
