use crate::cache::Cache;
use ethrex_common::{
    H256,
    types::{AccountUpdate, ELASTICITY_MULTIPLIER, Receipt},
};
use ethrex_levm::{
    db::{CacheDB, gen_db::GeneralizedDatabase},
    vm::VMType,
};
use ethrex_vm::{DynVmDatabase, Evm, EvmEngine, backends::levm::LEVM};
use eyre::Ok;
use std::sync::Arc;
use zkvm_interface::io::ProgramInput;

pub async fn exec(cache: Cache) -> eyre::Result<()> {
    let Cache {
        blocks,
        witness: db,
    } = cache;
    let input = ProgramInput {
        blocks,
        db,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        // The L2 specific fields (state_diff, blob_commitment, blob_proof)
        // will be filled by Default::default() if the 'l2' feature of
        // 'zkvm_interface' is active (due to workspace compilation).
        // If 'zkvm_interface' is compiled without 'l2' (e.g. standalone build),
        // these fields won't exist in ProgramInput, and ..Default::default()
        // will correctly not try to fill them.
        // A better solution would involve rethinking the `l2` feature or the
        // inclusion of this crate in the workspace.
        ..Default::default()
    };
    ethrex_prover_lib::execute(input).map_err(|e| eyre::Error::msg(e.to_string()))?;
    Ok(())
}

pub async fn prove(cache: Cache) -> eyre::Result<()> {
    let Cache {
        blocks,
        witness: db,
    } = cache;
    ethrex_prover_lib::prove(
        ProgramInput {
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
        },
        false,
    )
    .map_err(|e| eyre::Error::msg(e.to_string()))?;
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

    let vm_type = if l2 { VMType::L2 } else { VMType::L1 };

    let changes = {
        let store: Arc<DynVmDatabase> = Arc::new(Box::new(prover_db.clone()));
        let mut db = GeneralizedDatabase::new(store.clone(), CacheDB::new());
        LEVM::prepare_block(block, &mut db, vm_type)?;
        LEVM::get_state_transitions(&mut db)?
    };
    prover_db.apply_account_updates(&changes)?;

    for (tx, tx_sender) in block.body.get_transactions_with_sender()? {
        let mut vm = if l2 {
            Evm::new_for_l2(EvmEngine::LEVM, prover_db.clone())?
        } else {
            Evm::new_for_l1(EvmEngine::LEVM, prover_db.clone())
        };
        let (receipt, _) = vm.execute_tx(tx, &block.header, &mut remaining_gas, tx_sender)?;
        let account_updates = vm.get_state_transitions()?;
        prover_db.apply_account_updates(&account_updates)?;
        if tx.compute_hash() == tx_hash {
            return Ok((receipt, account_updates));
        }
    }
    Err(eyre::Error::msg("transaction not found inside block"))
}
