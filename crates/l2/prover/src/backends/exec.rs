use ethrex_blockchain::{validate_block, validate_gas_used};
use ethrex_common::types::{blob_from_bytes, kzg_commitment_to_versioned_hash};
use ethrex_common::{types::AccountUpdate, Address};
use ethrex_l2::utils::prover::proving_systems::{ProofCalldata, ProverType};
use ethrex_l2_sdk::calldata::Value;
use ethrex_vm::Evm;
use std::collections::HashMap;
use tracing::warn;
use zkvm_interface::{
    io::{ProgramInput, ProgramOutput},
    trie::{update_tries, verify_db},
};

#[cfg(feature = "l2")]
use ethrex_l2_common::{
    compute_deposit_logs_hash, compute_withdrawals_merkle_root, get_block_deposits,
    get_block_withdrawal_hashes,
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
    // TODO: add blob thingy

    // Tries used for validating initial and final state root
    let (mut state_trie, mut storage_tries) = db.get_tries()?;

    // Validate the initial state
    let initial_state_hash = state_trie.hash_no_commit();
    if initial_state_hash != parent_block_header.state_root {
        return Err("invalid initial state trie".to_string().into());
    }
    if !verify_db(&db, &state_trie, &storage_tries)? {
        return Err("invalid database".to_string().into());
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
        )?;

        // Execute block
        let mut vm = Evm::from_prover_db(db.clone());
        let result = vm.execute_block(&block)?;
        let receipts = result.receipts;
        let account_updates = vm.get_state_transitions()?;

        // Get L2 withdrawals and deposits for this block
        #[cfg(feature = "l2")]
        {
            let txs = block.body.transactions;
            let block_deposits = get_block_deposits(&txs);

            let txs_and_receipts: Vec<_> =
                txs.into_iter().zip(receipts.clone().into_iter()).collect();
            let block_withdrawal_hashes = get_block_withdrawal_hashes(&txs_and_receipts)?;

            let mut block_deposits_hashes = Vec::with_capacity(block_deposits.len());
            for deposit in &block_deposits {
                if let Some(hash) = deposit.get_deposit_hash() {
                    block_deposits_hashes.push(hash);
                } else {
                    return Err("Failed to get deposit hash for tx".to_string().into());
                }
            }
            withdrawal_hashes.extend(block_withdrawal_hashes);
            deposits_hashes.extend(
                block_deposits
                    .iter()
                    .filter_map(|tx| tx.get_deposit_hash())
                    .collect::<Vec<_>>(),
            );
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

        validate_gas_used(&receipts, &block.header)?;
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
    )?;

    // Calculate final state root hash and check
    let final_state_hash = state_trie.hash_no_commit();
    if final_state_hash != last_block_state_root {
        return Err("invalid final state trie".to_string().into());
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

    Ok(ProgramOutput {
        initial_state_hash,
        final_state_hash,
        #[cfg(feature = "l2")]
        withdrawals_merkle_root,
        #[cfg(feature = "l2")]
        deposit_logs_hash,
        #[cfg(feature = "l2")]
        blob_versioned_hash,
    })
}
