use crate::cache::Cache;
use ethrex_common::{
    H256,
    types::{
        AccountUpdate, ELASTICITY_MULTIPLIER, Receipt, block_execution_witness::GuestProgramState,
    },
};
use ethrex_levm::{db::gen_db::GeneralizedDatabase, vm::VMType};
use ethrex_prover_lib::backend::Backend;
use ethrex_rpc::debug::execution_witness::execution_witness_from_rpc_chain_config;
use ethrex_vm::{DynVmDatabase, Evm, GuestProgramStateWrapper, backends::levm::LEVM};
use eyre::Context;
use guest_program::input::ProgramInput;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

pub async fn exec(backend: Backend, cache: Cache) -> eyre::Result<()> {
    #[cfg(feature = "l2")]
    let input = get_l2_input(cache)?;
    #[cfg(not(feature = "l2"))]
    let input = get_l1_input(cache)?;

    // Use catch_unwind to capture panics
    let result = catch_unwind(AssertUnwindSafe(|| {
        ethrex_prover_lib::execute(backend, input)
    }));

    match result {
        Ok(exec_result) => {
            exec_result.map_err(|e| eyre::Error::msg(format!("Execution failed: {}", e)))?;
            Ok(())
        }
        Err(panic_info) => {
            // Try to extract meaningful error message from panic info
            let panic_msg = extract_panic_message(&panic_info);

            Err(eyre::Error::msg(format!(
                "Execution panicked: {}",
                panic_msg
            )))
        }
    }
}

pub async fn prove(backend: Backend, cache: Cache) -> eyre::Result<()> {
    #[cfg(feature = "l2")]
    let input = get_l2_input(cache)?;
    #[cfg(not(feature = "l2"))]
    let input = get_l1_input(cache)?;

    // Use catch_unwind to capture panics
    let result = catch_unwind(AssertUnwindSafe(|| {
        ethrex_prover_lib::prove(backend, input, false)
    }));

    match result {
        Ok(prove_result) => {
            prove_result.map_err(|e| eyre::Error::msg(format!("Proving failed: {}", e)))?;
            Ok(())
        }
        Err(panic_info) => {
            // Try to extract meaningful error message from panic info
            let panic_msg = extract_panic_message(&panic_info);

            Err(eyre::Error::msg(format!("Proving panicked: {}", panic_msg)))
        }
    }
}

pub async fn run_tx(cache: Cache, tx_hash: H256) -> eyre::Result<(Receipt, Vec<AccountUpdate>)> {
    let block = cache
        .blocks
        .first()
        .ok_or(eyre::Error::msg("missing block data"))?;

    let mut remaining_gas = block.header.gas_limit;

    let execution_witness = cache.witness;
    let network = cache.network;
    let chain_config = network
        .get_genesis()
        .map_err(|_| eyre::Error::msg("Failed to get genesis block"))?
        .config;

    let execution_witness = execution_witness_from_rpc_chain_config(
        execution_witness,
        chain_config,
        block.header.number,
    )
    .wrap_err("Failed to convert execution witness")?;

    let guest_program_state: GuestProgramState =
        execution_witness.try_into().map_err(eyre::Error::msg)?;

    let mut wrapped_db = GuestProgramStateWrapper::new(guest_program_state);

    #[cfg(feature = "l2")]
    let vm_type = VMType::L2;
    #[cfg(not(feature = "l2"))]
    let vm_type = VMType::L1;

    let changes = {
        let store: Arc<DynVmDatabase> = Arc::new(Box::new(wrapped_db.clone()));
        let mut db = GeneralizedDatabase::new(store.clone());
        LEVM::prepare_block(block, &mut db, vm_type)?;
        LEVM::get_state_transitions(&mut db)?
    };
    wrapped_db.apply_account_updates(&changes)?;

    for (tx, tx_sender) in block.body.get_transactions_with_sender()? {
        #[cfg(feature = "l2")]
        let mut vm = Evm::new_for_l2(wrapped_db.clone())?;
        #[cfg(not(feature = "l2"))]
        let mut vm = Evm::new_for_l1(wrapped_db.clone());
        let (receipt, _) = vm.execute_tx(tx, &block.header, &mut remaining_gas, tx_sender)?;
        let account_updates = vm.get_state_transitions()?;
        wrapped_db.apply_account_updates(&account_updates)?;
        if tx.hash() == tx_hash {
            return Ok((receipt, account_updates));
        }
    }

    Err(eyre::Error::msg("transaction not found inside block"))
}

#[cfg(not(feature = "l2"))]
fn get_l1_input(cache: Cache) -> eyre::Result<ProgramInput> {
    let Cache {
        blocks,
        witness: db,
        network,
        chain_config,
        l2_fields,
    } = cache;

    if l2_fields.is_some() {
        return Err(eyre::eyre!("Unexpected L2 fields in cache"));
    }
    if chain_config.is_some() {
        return Err(eyre::eyre!("Unexpected chain config in cache"));
    }
    let chain_config = network
        .get_genesis()
        .map_err(|_| eyre::Error::msg("Failed to get genesis block"))?
        .config;
    let first_block_number = blocks
        .first()
        .ok_or_else(|| eyre::eyre!("No blocks in cache"))?
        .header
        .number;

    let execution_witness =
        execution_witness_from_rpc_chain_config(db, chain_config, first_block_number)
            .wrap_err("Failed to convert execution witness")?;

    Ok(ProgramInput {
        blocks,
        execution_witness,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        // The L2 specific fields (blob_commitment, blob_proof)
        // will be filled by Default::default() if the 'l2' feature of
        // 'zkvm_interface' is active (due to workspace compilation).
        // If 'zkvm_interface' is compiled without 'l2' (e.g. standalone build),
        // these fields won't exist in ProgramInput, and ..Default::default()
        // will correctly not try to fill them.
        // A better solution would involve rethinking the `l2` feature or the
        // inclusion of this crate in the workspace.
        ..Default::default()
    })
}

/// Extract a meaningful error message from panic information.
fn extract_panic_message(panic_info: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic_info.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = panic_info.downcast_ref::<&str>() {
        s.to_string()
    } else {
        "Unknown panic occurred".to_string()
    }
}

#[cfg(feature = "l2")]
fn get_l2_input(cache: Cache) -> eyre::Result<ProgramInput> {
    let Cache {
        blocks,
        witness: db,
        chain_config,
        l2_fields,
        ..
    } = cache;

    let l2_fields = l2_fields.ok_or_else(|| eyre::eyre!("Missing L2 fields in cache"))?;
    let chain_config = chain_config.ok_or_else(|| eyre::eyre!("Missing chain config in cache"))?;

    let first_block_number = blocks
        .first()
        .ok_or_else(|| eyre::eyre!("No blocks in cache"))?
        .header
        .number;
    let execution_witness =
        execution_witness_from_rpc_chain_config(db, chain_config, first_block_number)
            .wrap_err("Failed to convert execution witness")?;

    Ok(ProgramInput {
        blocks,
        execution_witness,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        blob_commitment: l2_fields.blob_commitment,
        blob_proof: l2_fields.blob_proof,
    })
}
