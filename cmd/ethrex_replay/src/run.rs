use crate::cache::Cache;
use ethrex_common::{
    H256,
    types::{AccountUpdate, ELASTICITY_MULTIPLIER, Receipt},
};
use ethrex_levm::{db::gen_db::GeneralizedDatabase, vm::VMType};
use ethrex_prover_lib::backends::Backend;
use ethrex_vm::{DynVmDatabase, Evm, EvmEngine, ExecutionWitnessWrapper, backends::levm::LEVM};
use eyre::Ok;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};
use zkvm_interface::io::ProgramInput;

pub async fn exec(backend: Backend, cache: Cache) -> eyre::Result<()> {
    #[cfg(feature = "l2")]
    let input = get_l2_input(cache)?;
    #[cfg(not(feature = "l2"))]
    let input = get_l1_input(cache)?;

    ethrex_prover_lib::execute(backend, input).map_err(|e| eyre::Error::msg(e.to_string()))?;

    Ok(())
}

pub async fn prove(backend: Backend, cache: Cache) -> eyre::Result<()> {
    #[cfg(feature = "l2")]
    let input = get_l2_input(cache)?;
    #[cfg(not(feature = "l2"))]
    let input = get_l1_input(cache)?;

    catch_unwind(AssertUnwindSafe(|| {
        ethrex_prover_lib::prove(backend, input, false).map_err(|e| eyre::Error::msg(e.to_string()))
    }))
    .map_err(|_e| eyre::Error::msg("SP1 panicked while proving"))??;

    Ok(())
}

pub async fn run_tx(
    cache: Cache,
    tx_hash: H256,
    l2: bool,
) -> eyre::Result<(Receipt, Vec<AccountUpdate>)> {
    let block = cache
        .blocks
        .first()
        .ok_or(eyre::Error::msg("missing block data"))?;
    let mut remaining_gas = block.header.gas_limit;
    let mut prover_db = cache.witness;
    prover_db.rebuild_state_trie()?;
    let mut wrapped_db = ExecutionWitnessWrapper::new(prover_db);

    let vm_type = if l2 { VMType::L2 } else { VMType::L1 };

    let changes = {
        let store: Arc<DynVmDatabase> = Arc::new(Box::new(wrapped_db.clone()));
        let mut db = GeneralizedDatabase::new(store.clone());
        LEVM::prepare_block(block, &mut db, vm_type)?;
        LEVM::get_state_transitions(&mut db)?
    };
    wrapped_db.apply_account_updates(&changes)?;

    for (tx, tx_sender) in block.body.get_transactions_with_sender()? {
        let mut vm = if l2 {
            Evm::new_for_l2(EvmEngine::LEVM, wrapped_db.clone())?
        } else {
            Evm::new_for_l1(EvmEngine::LEVM, wrapped_db.clone())
        };
        let (receipt, _) = vm.execute_tx(tx, &block.header, &mut remaining_gas, tx_sender)?;
        let account_updates = vm.get_state_transitions()?;
        wrapped_db.apply_account_updates(&account_updates)?;
        if tx.hash() == tx_hash {
            return Ok((receipt, account_updates));
        }
    }
    Err(eyre::Error::msg("transaction not found inside block"))
}

fn get_l1_input(cache: Cache) -> eyre::Result<ProgramInput> {
    let Cache {
        blocks,
        witness: db,
        l2_fields,
    } = cache;

    if l2_fields.is_some() {
        return Err(eyre::eyre!("Unexpected L2 fields in cache"));
    }

    Ok(ProgramInput {
        blocks,
        db,
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

#[cfg(feature = "l2")]
fn get_l2_input(cache: Cache) -> eyre::Result<ProgramInput> {
    let Cache {
        blocks,
        witness: db,
        l2_fields,
    } = cache;

    let l2_fields = l2_fields.ok_or_else(|| eyre::eyre!("Missing L2 fields in cache"))?;

    Ok(ProgramInput {
        blocks,
        db,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        blob_commitment: l2_fields.blob_commitment,
        blob_proof: l2_fields.blob_proof,
    })
}
