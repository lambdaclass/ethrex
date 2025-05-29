use std::collections::HashMap;

use configfs_tsm::create_tdx_quote;

use std::time::Duration;
use tokio::time::sleep;

use ethrex_blockchain::{validate_block, validate_gas_used};
use ethrex_common::{types::AccountUpdate, Address, Bytes};
use ethrex_l2_sdk::calldata::{encode_tuple, Value};
use ethrex_l2_sdk::get_address_from_secret_key;
use ethrex_vm::Evm;
use zkvm_interface::{
    io::ProgramInput,
    trie::{update_tries, verify_db},
};

use keccak_hash::keccak;
use secp256k1::{generate_keypair, rand, Message, SecretKey};
mod sender;
use sender::{get_batch, submit_proof, submit_quote};

#[cfg(feature = "l2")]
use ethrex_l2_common::{
    get_block_deposits, get_block_withdrawal_hashes, compute_deposit_logs_hash, compute_withdrawals_merkle_root,
};
#[cfg(feature = "l2")]
use ethrex_common::types::{kzg_commitment_to_versioned_hash, blob_from_bytes};

use ethrex_l2::utils::prover::proving_systems::{ProofCalldata, ProverType};

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
    let ProgramInput {
        blocks,
        parent_block_header,
        mut db,
        elasticity_multiplier,
        #[cfg(feature = "l2")]
        state_diff,
        #[cfg(feature = "l2")]
        blob_commitment,
        #[cfg(feature = "l2")]
        blob_proof,
    } = input;
    // Tries used for validating initial and final state root
    let (mut state_trie, mut storage_tries) = db
        .get_tries()
        .map_err(|e| format!("Error getting tries: {e}"))?;

    // Validate the initial state
    let initial_state_hash = state_trie.hash_no_commit();
    if initial_state_hash != parent_block_header.state_root {
        return Err("invalid initial state trie".to_string());
    }
    if !verify_db(&db, &state_trie, &storage_tries)
        .map_err(|e| format!("Error verifying db: {e}"))?
    {
        return Err("invalid database".to_string());
    };

    let last_block = blocks.last().ok_or("empty batch".to_string())?;
    let last_block_state_root = last_block.header.state_root;
    let mut parent_header = parent_block_header;
    let mut acc_account_updates: HashMap<Address, AccountUpdate> = HashMap::new();

    #[cfg(feature = "l2")]
    let mut withdrawal_hashes = vec![];
    #[cfg(feature = "l2")]
    let mut deposits_hashes = vec![];

    for block in blocks {
        // Validate the block
        validate_block(
            &block,
            &parent_header,
            &db.chain_config,
            elasticity_multiplier,
        )
        .map_err(|e| format!("Error validating block: {e}"))?;

        // Execute block
        let mut vm = Evm::from_prover_db(db.clone());
        let result = vm
            .execute_block(&block)
            .map_err(|e| format!("Error executing block: {e}"))?;
        let receipts = result.receipts;
        let account_updates = vm
            .get_state_transitions()
            .map_err(|e| format!("Error getting transitions: {e}"))?;

        // Get L2 withdrawals and deposits for this block
        #[cfg(feature = "l2")]
        {
            let txs = block.body.transactions;
            let block_deposits = get_block_deposits(&txs);

            let txs_and_receipts: Vec<_> = txs.into_iter().zip(receipts.clone().into_iter()).collect();
            let block_withdrawal_hashes = get_block_withdrawal_hashes(&txs_and_receipts).expect("failed to retrieve withdrawal hashes");

            let mut block_deposit_hashes = Vec::with_capacity(block_deposits.len());
            for deposit in block_deposits {
                if let Some(hash) = deposit.get_deposit_hash() {
                    block_deposit_hashes.push(hash);
                } else {
                    return Err("Failed to get deposit hash for tx".to_string());
                }
            }
            withdrawal_hashes.extend(block_withdrawal_hashes);
            deposits_hashes.extend(block_deposit_hashes);
        }

        // Update db for the next block
        db.apply_account_updates(&account_updates);

        // Update acc_account_updates
        for account in account_updates {
            let address = account.address;
            if let Some(existing) = acc_account_updates.get_mut(&address) {
                existing.merge(account);
            } else {
                acc_account_updates.insert(address, account);
            }
        }

        validate_gas_used(&receipts, &block.header)
            .map_err(|e| format!("Error validating gas usage: {e}"))?;
        parent_header = block.header;
    }

    // Calculate account updates based on state diff
    #[cfg(feature = "l2")]
    let Ok(state_diff_updates) = state_diff.to_account_updates(&state_trie) else {
        return Err("Failed to calculate account updates from state diffs"
            .to_string()
            .into());
    };

    // Calculate L2 withdrawals root
    #[cfg(feature = "l2")]
    let Ok(withdrawals_merkle_root) = compute_withdrawals_merkle_root(withdrawal_hashes) else {
        return Err("Failed to calculate withdrawals merkle root"
            .to_string()
            .into());
    };

    // Calculate L2 deposits logs root
    #[cfg(feature = "l2")]
    let Ok(deposit_logs_hash) = compute_deposit_logs_hash(deposits_hashes) else {
        return Err("Failed to calculate deposits logs hash".to_string().into());
    };

    // Update state trie
    update_tries(
        &mut state_trie,
        &mut storage_tries,
        &acc_account_updates.values().cloned().collect::<Vec<_>>(),
    ).map_err(|e| format!("Error updating tries: {e}"))?;

    // Calculate final state root hash and check
    let final_state_hash = state_trie.hash_no_commit();
    if final_state_hash != last_block_state_root {
        return Err("invalid final state trie".to_string());
    }

    // Check state diffs are valid
    #[cfg(feature = "l2")]
    if state_diff_updates != acc_account_updates {
        return Err("invalid state diffs".to_string().into());
    }

    // Verify KZG blob proof
    #[cfg(feature = "l2")]
    let blob_versioned_hash = {
        use kzg_rs::{get_kzg_settings, Blob, Bytes48, KzgProof};

        let encoded_state_diff = state_diff
            .encode()
            .map_err(|e| format!("failed to encode state diff: {}", e))?;
        let blob_data = blob_from_bytes(encoded_state_diff)
            .map_err(|e| format!("failed to convert encoded state diff into blob data: {}", e))?;
        let blob = Blob::from_slice(&blob_data)
            .map_err(|_| "failed to convert blob data into Blob".to_string())?;

        let blob_proof_valid = KzgProof::verify_blob_kzg_proof(
            blob,
            &Bytes48::from_slice(&blob_commitment)
                .map_err(|_| "failed type conversion for blob commitment".to_string())?,
            &Bytes48::from_slice(&blob_proof)
                .map_err(|_| "failed type conversion for blob proof".to_string())?,
            &get_kzg_settings(),
        )
        .map_err(|e| {
            format!(
                "failed to verify blob proof (neither valid or invalid proof): {}",
                e
            )
        })?;

        if !blob_proof_valid {
            return Err("invalid blob proof".into());
        }

        kzg_commitment_to_versioned_hash(&blob_commitment)
    };

    let initial_hash_bytes = initial_state_hash.0.to_vec();
    let final_hash_bytes = final_state_hash.0.to_vec();
    #[cfg(feature = "l2")]
    let withdrawals_merkle_root_bytes = withdrawals_merkle_root.0.to_vec();
    #[cfg(feature = "l2")]
    let deposit_logs_hash_bytes = deposit_logs_hash.0.to_vec();
    #[cfg(feature = "l2")]
    let blob_versioned_hash_bytes = blob_versioned_hash.0.to_vec();

    let data = vec![
        Value::FixedBytes(initial_hash_bytes.into()),
        Value::FixedBytes(final_hash_bytes.into()),
        #[cfg(feature = "l2")]
        Value::FixedBytes(withdrawals_merkle_root_bytes.into()),
        #[cfg(feature = "l2")]
        Value::FixedBytes(deposit_logs_hash_bytes.into()),
        #[cfg(feature = "l2")]
        Value::FixedBytes(blob_versioned_hash_bytes.into()),
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
    let calldata = vec![Value::Bytes(output.into()), Value::Bytes(signature.into())];
    submit_proof(
        batch_number,
        ProofCalldata {
            prover_type: ProverType::TDX,
            calldata,
        },
    )
    .await?;
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
