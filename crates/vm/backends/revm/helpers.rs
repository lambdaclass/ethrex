use ethrex_common::{
    types::{tx_fields::AccessList, BlockHeader, GenericTransaction, INITIAL_BASE_FEE},
    Address, H256,
};
use revm::{
    inspector_handle_register,
    primitives::{BlockEnv, TxEnv},
    Evm,
};
// Rename imported types for clarity
use revm_primitives::{AccessList as RevmAccessList, SpecId};

use crate::{errors::EvmError, execution_result::ExecutionResult};

use super::{
    access_list_inspector, block_env, db::EvmState, run_without_commit, tx_env_from_generic,
};

// Executes a single GenericTransaction, doesn't commit the result or perform state transitions
pub fn simulate_tx_from_generic(
    tx: &GenericTransaction,
    header: &BlockHeader,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<ExecutionResult, EvmError> {
    let block_env = block_env(header, spec_id);
    let tx_env = tx_env_from_generic(tx, header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE));
    run_without_commit(tx_env, block_env, state, spec_id)
}

/// Runs the transaction and returns the access list and estimated gas use (when running the tx with said access list)
pub fn create_access_list(
    tx: &GenericTransaction,
    header: &BlockHeader,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<(ExecutionResult, AccessList), EvmError> {
    let mut tx_env = tx_env_from_generic(tx, header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE));
    let block_env = block_env(header, spec_id);
    // Run tx with access list inspector

    let (execution_result, access_list) =
        create_access_list_inner(tx_env.clone(), block_env.clone(), state, spec_id)?;

    // Run the tx with the resulting access list and estimate its gas used
    let execution_result = if execution_result.is_success() {
        tx_env.access_list.extend(access_list.0.clone());

        run_without_commit(tx_env, block_env, state, spec_id)?
    } else {
        execution_result
    };
    let access_list: Vec<(Address, Vec<H256>)> = access_list
        .iter()
        .map(|item| {
            (
                Address::from_slice(item.address.0.as_slice()),
                item.storage_keys
                    .iter()
                    .map(|v| H256::from_slice(v.as_slice()))
                    .collect(),
            )
        })
        .collect();
    Ok((execution_result, access_list))
}

/// Runs the transaction and returns the access list for it
fn create_access_list_inner(
    tx_env: TxEnv,
    block_env: BlockEnv,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<(ExecutionResult, RevmAccessList), EvmError> {
    let mut access_list_inspector = access_list_inspector(&tx_env)?;
    #[allow(unused_mut)]
    let mut evm_builder = Evm::builder()
        .with_block_env(block_env)
        .with_tx_env(tx_env)
        .with_spec_id(spec_id)
        .modify_cfg_env(|env| {
            env.disable_base_fee = true;
            env.disable_block_gas_limit = true
        })
        .with_external_context(&mut access_list_inspector);
    let tx_result = {
        match state {
            EvmState::Store(db) => {
                let mut evm = evm_builder
                    .with_db(db)
                    .append_handler_register(inspector_handle_register)
                    .build();
                evm.transact().map_err(EvmError::from)?
            }
            EvmState::Execution(db) => {
                let mut evm = evm_builder
                    .with_db(db)
                    .append_handler_register(inspector_handle_register)
                    .build();
                evm.transact().map_err(EvmError::from)?
            }
        }
    };

    let access_list = access_list_inspector.into_access_list();
    Ok((tx_result.result.into(), access_list))
}
