use std::collections::HashMap;

use configfs_tsm::create_tdx_quote;

use std::time::Duration;
use tokio::time::sleep;

use ethrex_blockchain::{validate_block, validate_gas_used};
use ethrex_common::{types::AccountUpdate, Address, Bytes};
use ethrex_l2_sdk::calldata::{encode_tuple, Value};
use ethrex_l2_sdk::get_address_from_secret_key;
use ethrex_vm::Evm;
#[cfg(feature = "l2")]
use zkvm_interface::deposits::{get_block_deposits, get_deposit_hash};
#[cfg(feature = "l2")]
use zkvm_interface::withdrawals::{get_block_withdrawals, get_withdrawals_merkle_root};
use zkvm_interface::{
    io::ProgramInput,
    trie::{update_tries, verify_db},
};

use keccak_hash::keccak;
use secp256k1::{generate_keypair, rand, Message, SecretKey};
mod sender;
use sender::{get_batch, submit_proof, submit_quote};

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
    let mut withdrawals = vec![];
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
            let block_withdrawals = get_block_withdrawals(&block.body.transactions, &receipts)
                .map_err(|e| format!("Error getting block withdrawls: {e}"))?;
            let block_deposits = get_block_deposits(&block.body.transactions);
            let mut block_deposits_hashes = Vec::with_capacity(block_deposits.len());
            for deposit in block_deposits {
                if let Some(hash) = deposit.get_deposit_hash() {
                    block_deposits_hashes.push(hash);
                } else {
                    return Err("Failed to get deposit hash for tx".to_string());
                }
            }
            withdrawals.extend(block_withdrawals);
            deposits_hashes.extend(block_deposits_hashes);
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

    // Calculate L2 withdrawals root
    #[cfg(feature = "l2")]
    let Ok(withdrawals_merkle_root) = get_withdrawals_merkle_root(withdrawals) else {
        return Err("Failed to calculate withdrawals merkle root".to_string());
    };

    // Calculate L2 deposits logs root
    #[cfg(feature = "l2")]
    let Ok(deposit_logs_hash) = get_deposit_hash(deposits_hashes) else {
        return Err("Failed to calculate deposits logs hash".to_string());
    };

    // Update state trie
    let acc_account_updates: Vec<AccountUpdate> = acc_account_updates.values().cloned().collect();
    update_tries(&mut state_trie, &mut storage_tries, &acc_account_updates)
        .map_err(|e| format!("Error updating tries: {e}"))?;

    // Calculate final state root hash and check
    let final_state_hash = state_trie.hash_no_commit();
    if final_state_hash != last_block_state_root {
        return Err("invalid final state trie".to_string());
    }

    let initial_hash_bytes = initial_state_hash.0.to_vec();
    let final_hash_bytes = final_state_hash.0.to_vec();
    #[cfg(feature = "l2")]
    let withdrawals_merkle_root_bytes = withdrawals_merkle_root.0.to_vec();
    #[cfg(feature = "l2")]
    let deposit_logs_hash_bytes = deposit_logs_hash.0.to_vec();

    let data = vec![
        Value::FixedBytes(initial_hash_bytes.into()),
        Value::FixedBytes(final_hash_bytes.into()),
        #[cfg(feature = "l2")]
        Value::FixedBytes(withdrawals_merkle_root_bytes.into()),
        #[cfg(feature = "l2")]
        Value::FixedBytes(deposit_logs_hash_bytes.into()),
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
