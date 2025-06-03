use crate::cache::Cache;
use ethrex_common::types::{AccountUpdate, Receipt, ELASTICITY_MULTIPLIER};
use ethrex_levm::db::{gen_db::GeneralizedDatabase, CacheDB};
use ethrex_vm::{backends::levm::LEVM, DynVmDatabase, Evm, EvmEngine};
use eyre::Ok;
use std::sync::Arc;
use zkvm_interface::io::ProgramInput;

pub async fn exec(cache: Cache) -> eyre::Result<()> {
    let Cache {
        blocks,
        parent_block_header,
        db,
    } = cache;
    let input = ProgramInput {
        blocks,
        parent_block_header,
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

pub async fn prove(cache: Cache) -> eyre::Result<String> {
    let Cache {
        blocks,
        parent_block_header,
        db,
    } = cache;
    let out = ethrex_prover_lib::prove(ProgramInput {
        blocks,
        parent_block_header,
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
    })
    .map_err(|e| eyre::Error::msg(e.to_string()))?;
    #[cfg(feature = "sp1")]
    return Ok(format!("{out:#?}"));
    #[cfg(not(feature = "sp1"))]
    Ok(serde_json::to_string(&out.0)?)
}

pub async fn run_tx(cache: Cache, tx_id: &str) -> eyre::Result<(Receipt, Vec<AccountUpdate>)> {
    let block = cache
        .blocks
        .first()
        .ok_or(eyre::Error::msg("missing block data"))?;
    let mut remaining_gas = block.header.gas_limit;
    let mut prover_db = cache.db;

    // GeneralizedDatabase::new explicitly requires an Arc
    // TODO: refactor GeneralizedDatabase and/or Database to avoid this
    {
        let store: Arc<DynVmDatabase> = Arc::new(Box::new(prover_db.clone()));
        let mut db = GeneralizedDatabase::new(store.clone(), CacheDB::new());
        LEVM::prepare_block(block, &mut db)?;
    }

    for (tx, tx_sender) in block.body.get_transactions_with_sender() {
        let mut vm = Evm::new(EvmEngine::LEVM, prover_db.clone());
        let (receipt, _) = vm.execute_tx(tx, &block.header, &mut remaining_gas, tx_sender)?;
        let account_updates = vm.get_state_transitions()?;
        prover_db.apply_account_updates(&account_updates);
        if format!("0x{:x}", tx.compute_hash()) == tx_id {
            return Ok((receipt, account_updates));
        }
    }
    Err(eyre::Error::msg("transaction not found inside block"))
}
