use ethrex_common::types::{ChainConfig, Fork, TxType};
use ethrex_common::{
    Address, H256,
    types::{BlockHeader, GenericTransaction, INITIAL_BASE_FEE, tx_fields::AccessList},
};
use revm::{ExecuteEvm, MainBuilder, MainContext};
use revm::{
    context::{BlockEnv, TxEnv},
    context_interface::transaction::AccessList as RevmAccessList,
};

// Rename imported types for clarity
use revm_primitives::hardfork::SpecId;

use crate::{backends::revm::db::EvmState, errors::EvmError, execution_result::ExecutionResult};

use super::{access_list_inspector, block_env, run_without_commit, tx_env_from_generic};

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
        tx_env.access_list.0.extend(access_list.0.clone());

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
    let access_list_inspector = access_list_inspector(&tx_env)?;
    let mut evm_context = revm::context::Context::mainnet()
        .with_block(block_env)
        .with_db(&mut state.inner);
    evm_context.modify_cfg(|cfg| {
        cfg.spec = spec_id;
        cfg.disable_base_fee = true;
        cfg.disable_block_gas_limit = true
    });
    let mut evm = evm_context.build_mainnet_with_inspector(access_list_inspector);
    let tx_result = evm.transact(tx_env)?;
    let access_list = evm.inspector.into_access_list();
    Ok((tx_result.result.into(), access_list))
}

/// Returns the spec id according to the block timestamp and the stored chain config
/// WARNING: Assumes at least Merge fork is active
pub fn spec_id(chain_config: &ChainConfig, block_timestamp: u64) -> SpecId {
    fork_to_spec_id(chain_config.get_fork(block_timestamp))
}

pub fn fork_to_spec_id(fork: Fork) -> SpecId {
    match fork {
        Fork::Frontier => SpecId::FRONTIER,
        Fork::FrontierThawing => SpecId::FRONTIER_THAWING,
        Fork::Homestead => SpecId::HOMESTEAD,
        Fork::DaoFork => SpecId::DAO_FORK,
        Fork::Tangerine => SpecId::TANGERINE,
        Fork::SpuriousDragon => SpecId::SPURIOUS_DRAGON,
        Fork::Byzantium => SpecId::BYZANTIUM,
        Fork::Constantinople => SpecId::CONSTANTINOPLE,
        Fork::Petersburg => SpecId::PETERSBURG,
        Fork::Istanbul => SpecId::ISTANBUL,
        Fork::MuirGlacier => SpecId::MUIR_GLACIER,
        Fork::Berlin => SpecId::BERLIN,
        Fork::London => SpecId::LONDON,
        Fork::ArrowGlacier => SpecId::ARROW_GLACIER,
        Fork::GrayGlacier => SpecId::GRAY_GLACIER,
        Fork::Paris => SpecId::MERGE,
        Fork::Shanghai => SpecId::SHANGHAI,
        Fork::Cancun => SpecId::CANCUN,
        Fork::Prague => SpecId::PRAGUE,
        Fork::Osaka => SpecId::OSAKA,
    }
}

pub fn infer_generic_tx_type(tx: &GenericTransaction) -> TxType {
    if tx.authorization_list.is_some() {
        TxType::EIP7702
    } else if !tx.blob_versioned_hashes.is_empty() {
        TxType::EIP4844
    } else if !tx.access_list.is_empty() {
        TxType::EIP2930
    } else if tx.max_priority_fee_per_gas.is_some() {
        TxType::EIP1559
    } else {
        TxType::Legacy
    }
}
