use configfs_tsm::create_tdx_quote;

use std::time::Duration;
use tokio::time::sleep;

use ethrex_common::Bytes;
use ethrex_l2_sdk::calldata::{encode_tuple, Value};
use ethrex_l2_sdk::get_address_from_secret_key;
use zkvm_interface::io::ProgramInput;

use keccak_hash::keccak;
use secp256k1::{generate_keypair, rand, Message, SecretKey};
mod sender;
use sender::{get_batch, submit_proof, submit_quote};

use ethrex_l2::utils::prover::proving_systems::{BatchProof, ProofCalldata, ProverType};

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

    let msg_signature_recovery_id = msg_signature_recovery_id.to_i32() + 27;

    [&msg_signature[..], &[msg_signature_recovery_id as u8]].concat()
}

fn calculate_transition(input: ProgramInput) -> Result<Vec<u8>, String> {
    let output = zkvm_interface::execution::execution_program(input).map_err(|e| e.to_string())?;

    let initial_hash_bytes = output.initial_state_hash.0.to_vec();
    let final_hash_bytes = output.final_state_hash.0.to_vec();
    let last_block_hash_bytes = output.last_block_hash.0.to_vec();
    #[cfg(feature = "l2")]
    let l1messages_merkle_root_bytes = output.l1messages_merkle_root.0.to_vec();
    #[cfg(feature = "l2")]
    let deposit_logs_hash_bytes = output.deposit_logs_hash.0.to_vec();
    #[cfg(feature = "l2")]
    let blob_versioned_hash_bytes = output.blob_versioned_hash.0.to_vec();

    let data = vec![
        Value::FixedBytes(initial_hash_bytes.into()),
        Value::FixedBytes(final_hash_bytes.into()),
        #[cfg(feature = "l2")]
        Value::FixedBytes(l1messages_merkle_root_bytes.into()),
        #[cfg(feature = "l2")]
        Value::FixedBytes(deposit_logs_hash_bytes.into()),
        #[cfg(feature = "l2")]
        Value::FixedBytes(blob_versioned_hash_bytes.into()),
        Value::FixedBytes(last_block_hash_bytes.into()),
    ]
    .clone();
    let bytes = encode_tuple(&data).map_err(|e| format!("Error packing data: {e}"))?;
    Ok(bytes)
}

fn get_quote(private_key: &SecretKey) -> Result<Bytes, String> {
    let address = get_address_from_secret_key(private_key)
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

async fn do_loop(private_key: &SecretKey) -> Result<u64, String> {
    let (batch_number, input) = get_batch().await?;
    let output = calculate_transition(input)?;
    let signature = sign_eip191(&output, private_key);
    let calldata = ProofCalldata {
        prover_type: ProverType::TDX,
        calldata: vec![Value::Bytes(output.into()), Value::Bytes(signature.into())],
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
    while let Err(err) = setup(&private_key).await {
        println!("Error sending quote: {}", err);
        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
    loop {
        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        match do_loop(&private_key).await {
            Ok(batch_number) => println!("Processed batch {}", batch_number),
            Err(err) => println!("Error: {}", err),
        };
    }
}
