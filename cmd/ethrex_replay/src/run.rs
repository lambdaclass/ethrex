use crate::cache::{Cache, L2Fields};
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

/// Returns the input based on whether the feature "l2" is enabled or not.
/// If the feature is enabled, it includes L2 fields (blob commitment and proof).
fn get_input(cache: Cache) -> eyre::Result<ProgramInput> {
    let input = {
        cfg_if::cfg_if! {
            if #[cfg(not(feature = "l2"))] {
                let Cache {
                    blocks,
                    witness: db,
                    l2_fields: None,
                } = cache;

                ProgramInput {
                    blocks,
                    db,
                    elasticity_multiplier: ELASTICITY_MULTIPLIER,
                }
            } else {
                let Cache {
                    blocks,
                    witness: db,
                    l2_fields: Some(L2Fields {
                        blob_commitment,
                        blob_proof,
                    }),
                } = cache else {
                    return Err(eyre::Error::msg("missing L2 fields in cache"));
                };

                ProgramInput {
                    blocks,
                    db,
                    elasticity_multiplier: ELASTICITY_MULTIPLIER,
                    blob_commitment,
                    blob_proof,
                }
            }
        }
    };

    Ok(input)
}
