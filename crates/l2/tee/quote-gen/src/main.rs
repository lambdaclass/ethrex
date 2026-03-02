mod sender;

use configfs_tsm::create_tdx_quote;
use ethrex_common::Bytes;
use ethrex_common::utils::keccak;
use ethrex_l2_common::{
    calldata::Value,
    prover::{BatchProof, ProofCalldata, ProverType},
    utils::get_address_from_secret_key,
};
use ethrex_guest_program::input::ProgramInput;
use secp256k1::{Message, SecretKey, generate_keypair, rand};
use sender::{get_batch, submit_proof, submit_quote};
use std::time::Duration;
use tokio::time::sleep;

/// Returns the git commit hash of the current build.
fn get_git_commit_hash() -> String {
    env!("VERGEN_GIT_SHA").to_string()
}

const POLL_INTERVAL_MS: u64 = 5000;

fn sign_eip191(msg: &[u8], private_key: &SecretKey) -> Vec<u8> {
    let payload = [
        b"\x19Ethereum Signed Message:\n",
        msg.len().to_string().as_bytes(),
        msg,
    ]
    .concat();

    let signed_msg = secp256k1::SECP256K1.sign_ecdsa_recoverable(
        &Message::from_digest(*keccak(&payload).as_fixed_bytes()),
        private_key,
    );

    let (msg_signature_recovery_id, msg_signature) = signed_msg.serialize_compact();

    let msg_signature_recovery_id = Into::<i32>::into(msg_signature_recovery_id) + 27;

    [&msg_signature[..], &[msg_signature_recovery_id as u8]].concat()
}

fn calculate_transition(input: ProgramInput) -> Result<Vec<u8>, String> {
    let output = ethrex_guest_program::execution::execution_program(input).map_err(|e| e.to_string())?;

    Ok(output.encode())
}

fn get_quote(private_key: &SecretKey) -> Result<Bytes, String> {
    let address = get_address_from_secret_key(&private_key.secret_bytes())
        .map_err(|e| format!("Error deriving address: {e}"))?;
    let mut digest_slice = [0u8; 64];
    digest_slice
        .split_at_mut(20)
        .0
        .copy_from_slice(address.as_bytes());
    create_tdx_quote(digest_slice)
        .or_else(|err| {
            println!("Error creating quote: {err}");
            Ok(address.as_bytes().into())
        })
        .map(Bytes::from)
}

async fn do_loop(private_key: &SecretKey, commit_hash: String) -> Result<u64, String> {
    let (batch_number, input) = get_batch(commit_hash).await?;
    let output = calculate_transition(input)?;
    let signature = sign_eip191(&output, private_key);
    let calldata = ProofCalldata {
        prover_type: ProverType::TDX,
        calldata: vec![Value::Bytes(signature.into())],
    };

    submit_proof(batch_number, BatchProof::ProofCalldata(calldata)).await?;
    Ok(batch_number)
}

async fn setup(private_key: &SecretKey) -> Result<(), String> {
    let quote = get_quote(private_key)?;
    println!("Sending quote {}", hex::encode(&quote));
    submit_quote(quote).await?;
    Ok(())
}

#[tokio::main]
async fn main() {
    let (private_key, _) = generate_keypair(&mut rand::rngs::OsRng);
    let commit_hash = get_git_commit_hash();
    while let Err(err) = setup(&private_key).await {
        println!("Error sending quote: {}", err);
        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
    loop {
        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        match do_loop(&private_key, commit_hash.clone()).await {
            Ok(batch_number) => println!("Processed batch {}", batch_number),
            Err(err) => println!("Error: {}", err),
        };
    }
}
