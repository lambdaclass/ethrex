use crate::cache::Cache;
use ethrex_common::types::{AccountUpdate, Receipt, ELASTICITY_MULTIPLIER};
use ethrex_levm::db::{gen_db::GeneralizedDatabase, CacheDB};
use ethrex_vm::{backends::levm::LEVM, Evm};
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
    let mut store = {
        let store = Arc::new(cache.db);
        let mut db = GeneralizedDatabase::new(store.clone(), CacheDB::new());
        LEVM::prepare_block(block, &mut db)?;
        drop(db);
        // TODO: refactor GeneralizedDatabase and Database to avoid this
        Arc::into_inner(store).ok_or(eyre::Error::msg("couldn't get store out of Arc<>"))?
    };
    for (tx, tx_sender) in block.body.get_transactions_with_sender() {
        let mut vm = Evm::from_prover_db(store.clone());
        let (receipt, _) = vm.execute_tx(tx, &block.header, &mut remaining_gas, tx_sender)?;
        let account_updates = vm.get_state_transitions()?;
        store.apply_account_updates(&account_updates);
        if format!("0x{:x}", tx.compute_hash()) == tx_id {
            return Ok((receipt, account_updates));
        }
    }
    Err(eyre::Error::msg("transaction not found inside block"))
}
