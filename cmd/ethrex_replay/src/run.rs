use crate::cache::Cache;
use ethrex_common::{
    H256,
    types::{AccountUpdate, ELASTICITY_MULTIPLIER, Receipt},
};
use ethrex_levm::{
    db::{CacheDB, gen_db::GeneralizedDatabase},
    vm::VMType,
};
use ethrex_vm::{DynVmDatabase, Evm, EvmEngine, ExecutionWitnessWrapper, backends::levm::LEVM};
use eyre::Ok;
use std::sync::Arc;
use zkvm_interface::io::ProgramInput;

pub async fn exec(cache: Cache) -> eyre::Result<()> {
    let input = get_input(cache)?;
    ethrex_prover_lib::execute(input).map_err(|e| eyre::Error::msg(e.to_string()))?;
    Ok(())
}

pub async fn prove(cache: Cache) -> eyre::Result<()> {
    let input = get_input(cache)?;
    ethrex_prover_lib::prove(input, false).map_err(|e| eyre::Error::msg(e.to_string()))?;
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
    prover_db.rebuild_tries()?;
    let mut wrapped_db = ExecutionWitnessWrapper::new(prover_db);

    let vm_type = if l2 { VMType::L2 } else { VMType::L1 };

    let changes = {
        let store: Arc<DynVmDatabase> = Arc::new(Box::new(wrapped_db.clone()));
        let mut db = GeneralizedDatabase::new(store.clone(), CacheDB::new());
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
        if tx.compute_hash() == tx_hash {
            return Ok((receipt, account_updates));
        }
    }
    Err(eyre::Error::msg("transaction not found inside block"))
}

/// Returns the input based on whether the feature "l2" is enabled or not.
/// If the feature is enabled, it includes L2 fields (blob commitment and proof).
fn get_input(cache: Cache) -> eyre::Result<ProgramInput> {
    let Cache {
        blocks,
        witness: db,
        l2_fields,
    } = cache;

    #[cfg(not(feature = "l2"))]
    {
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
    {
        let l2_fields = l2_fields.ok_or_else(|| eyre::eyre!("Missing L2 fields in cache"))?;

        Ok(ProgramInput {
            blocks,
            db,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            blob_commitment: l2_fields.blob_commitment,
            blob_proof: l2_fields.blob_proof,
        })
    }
}
