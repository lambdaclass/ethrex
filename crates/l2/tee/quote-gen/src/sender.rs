use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use zkvm_interface::io::ProgramInput;

use ethrex_l2::sequencer::proof_coordinator::ProofData;
use ethrex_l2_common::prover::{BatchProof, ProverType};

use ethrex_common::Bytes;

const PROOF_COORDINATOR_URL_ENV: &str = "ETHREX_TDX_PROOF_COORDINATOR_URL";
const LOCAL_PROOF_COORDINATOR_URL: &str = "localhost:3900";

pub async fn get_batch(commit_hash: String) -> Result<(u64, ProgramInput), String> {
    let batch = connect_to_prover_server_wr(&ProofData::BatchRequest {
        commit_hash: commit_hash.clone(),
    })
    .await
    .map_err(|e| format!("Failed to get Response: {e}"))?;
    match batch {
        ProofData::BatchResponse {
            batch_number,
            input,
        } => match (batch_number, input) {
            (Some(batch_number), Some(input)) => Ok((
                batch_number,
                ProgramInput {
                    blocks: input.blocks,
                    db: input.db,
                    elasticity_multiplier: input.elasticity_multiplier,
                    #[cfg(feature = "l2")]
                    blob_commitment: input.blob_commitment,
                    #[cfg(feature = "l2")]
                    blob_proof: input.blob_proof,
                },
            )),
            _ => Err("No blocks to prove.".to_owned()),
        },
        ProofData::InvalidCodeVersion {
            commit_hash: server_code_version,
        } => Err(format!(
            "Invalid code version received. Server code: {}, Prover code: {}",
            server_code_version, commit_hash
        )),
        _ => Err("Expecting ProofData::Response".to_owned()),
    }
}

pub async fn submit_proof(batch_number: u64, batch_proof: BatchProof) -> Result<u64, String> {
    let submit = ProofData::proof_submit(batch_number, batch_proof);

    let submit_ack = connect_to_prover_server_wr(&submit)
        .await
        .map_err(|e| format!("Failed to get SubmitAck: {e}"))?;

    match submit_ack {
        ProofData::ProofSubmitACK { batch_number } => Ok(batch_number),
        _ => Err("Expecting ProofData::SubmitAck".to_owned()),
    }
}

pub async fn submit_quote(quote: Bytes) -> Result<(), String> {
    let setup = ProofData::prover_setup(ProverType::TDX, quote);

    let setup_ack = connect_to_prover_server_wr(&setup)
        .await
        .map_err(|e| format!("Failed to get ProverSetupAck: {e}"))?;

    match setup_ack {
        ProofData::ProverSetupACK => Ok(()),
        _ => Err("Expecting ProofData::ProverSetupACK".to_owned()),
    }
}

async fn connect_to_prover_server_wr(
    write: &ProofData,
) -> Result<ProofData, Box<dyn std::error::Error>> {
    let addr =
        std::env::var(PROOF_COORDINATOR_URL_ENV).unwrap_or(LOCAL_PROOF_COORDINATOR_URL.to_string());
    let mut stream = TcpStream::connect(addr).await?;

    stream.write_all(&serde_json::to_vec(&write)?).await?;
    stream.shutdown().await?;

    let mut buffer = Vec::new();
    stream.read_to_end(&mut buffer).await?;

    let response: Result<ProofData, _> = serde_json::from_slice(&buffer);
    Ok(response?)
}
