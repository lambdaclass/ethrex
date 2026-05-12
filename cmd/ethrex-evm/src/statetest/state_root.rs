use std::{collections::BTreeMap, sync::Arc};

use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, H256, U256,
    types::{Account, AccountInfo, AccountUpdate, Genesis, GenesisAccount},
};
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::DynVmDatabase;
use rustc_hash::FxHashMap;

/// Given a pre-state account map and a set of post-execution updates, returns
/// the post-state root by applying them to an in-memory Store. Returns
/// `eyre::Result<H256>` for caller-friendly error reporting.
///
/// # How it works
///
/// 1. Converts `pre_state` into a [`Genesis`] alloc and creates an in-memory [`Store`].
/// 2. Calls `store.add_initial_state(genesis).await` (async) to commit the pre-state trie and
///    obtain the genesis block hash.
/// 3. Calls `store.apply_account_updates_batch(block_hash, updates)` (sync) to apply the
///    post-execution deltas and compute the new state root from the trie.
/// 4. Returns the `state_trie_hash` from the returned [`AccountUpdatesList`].
///
/// The tokio runtime is created and discarded inside this function; the public signature stays
/// synchronous for ergonomics.
pub fn compute_post_state_root(
    pre_state: &FxHashMap<Address, Account>,
    updates: &[AccountUpdate],
) -> eyre::Result<H256> {
    let genesis = build_genesis(pre_state);

    let rt = tokio::runtime::Runtime::new()?;
    let (store, block_hash) = rt.block_on(async {
        let mut store =
            Store::new("./temp", EngineType::InMemory).map_err(|e| eyre::eyre!("{e}"))?;
        store
            .add_initial_state(genesis.clone())
            .await
            .map_err(|e| eyre::eyre!("{e}"))?;
        let block_hash = genesis.get_block().hash();
        Ok::<_, eyre::Error>((store, block_hash))
    })?;

    let result = store
        .apply_account_updates_batch(block_hash, updates)
        .map_err(|e| eyre::eyre!("{e}"))?
        .ok_or_else(|| {
            eyre::eyre!("apply_account_updates_batch: state trie not found for genesis block hash")
        })?;

    Ok(result.state_trie_hash)
}

/// Builds a minimal [`Genesis`] whose alloc matches the given account map.
///
/// The chain config uses Cancun-era activation times of 0 so that the genesis block
/// header is valid across all common forks. The genesis timestamp is 0.
fn build_genesis(pre_state: &FxHashMap<Address, Account>) -> Genesis {
    let alloc: BTreeMap<Address, GenesisAccount> = pre_state
        .iter()
        .map(|(addr, account)| {
            let genesis_account = account_to_genesis_account(account);
            (*addr, genesis_account)
        })
        .collect();

    // Mirror tooling/ef_tests/state/types.rs `Genesis::from(&EFTest)`: leave
    // the chain config at its `Default` (all-forks-inactive). LEVM's
    // execution-time `EVMConfig` is what drives fork-specific behavior; a
    // bespoke chain config here can leak Amsterdam/Prague checks into a
    // Shanghai run (observed: +32 gas overhead on a 21000-gas transfer).
    Genesis {
        alloc,
        gas_limit: 30_000_000,
        ..Default::default()
    }
}

/// Converts an [`Account`] back to a [`GenesisAccount`] for inclusion in a genesis alloc.
fn account_to_genesis_account(account: &Account) -> GenesisAccount {
    let storage: BTreeMap<U256, U256> = account
        .storage
        .iter()
        .map(|(k, v)| (U256::from_big_endian(k.as_bytes()), *v))
        .collect();

    GenesisAccount {
        code: account.code.bytecode.clone(),
        storage,
        balance: account.info.balance,
        nonce: account.info.nonce,
    }
}

/// Returns a [`ChainConfig`] with all common forks activated at block 0 / timestamp 0.
///
/// Exposed as `pub` so Phase 4 can reuse it when building a `Genesis` from a `StateTest`.
pub fn minimal_chain_config() -> ethrex_common::types::ChainConfig {
    use ethrex_common::types::ChainConfig;
    ChainConfig {
        chain_id: 1,
        homestead_block: Some(0),
        dao_fork_block: Some(0),
        dao_fork_support: true,
        eip150_block: Some(0),
        eip155_block: Some(0),
        eip158_block: Some(0),
        byzantium_block: Some(0),
        constantinople_block: Some(0),
        petersburg_block: Some(0),
        istanbul_block: Some(0),
        muir_glacier_block: Some(0),
        berlin_block: Some(0),
        london_block: Some(0),
        arrow_glacier_block: Some(0),
        gray_glacier_block: Some(0),
        merge_netsplit_block: Some(0),
        shanghai_time: Some(0),
        cancun_time: Some(0),
        prague_time: Some(0),
        terminal_total_difficulty: Some(0),
        terminal_total_difficulty_passed: true,
        ..Default::default()
    }
}

/// Builds a [`GeneralizedDatabase`] from a genesis + in-memory store.
///
/// This is a convenience used in tests to set up the pre-state for LEVM execution.
/// Phase 4 will use an equivalent inline construction when wiring the full VM pipeline.
pub fn build_generalized_db(store: Store, genesis: &Genesis) -> eyre::Result<GeneralizedDatabase> {
    let block_header = genesis.get_block().header;
    let vm_db: DynVmDatabase =
        Box::new(StoreVmDatabase::new(store, block_header).map_err(|e| eyre::eyre!("{e}"))?);
    Ok(GeneralizedDatabase::new(Arc::new(vm_db)))
}

/// Thin wrapper used in tests: sets up an in-memory store from `pre_state` and returns both the
/// store and the genesis block hash, without consuming the tokio runtime.
pub async fn setup_store(
    pre_state: &FxHashMap<Address, Account>,
) -> eyre::Result<(Store, H256, Genesis)> {
    let genesis = build_genesis(pre_state);
    let mut store = Store::new("./temp", EngineType::InMemory).map_err(|e| eyre::eyre!("{e}"))?;
    store
        .add_initial_state(genesis.clone())
        .await
        .map_err(|e| eyre::eyre!("{e}"))?;
    let block_hash = genesis.get_block().hash();
    Ok((store, block_hash, genesis))
}

/// Returns a minimal [`AccountInfo`] with the given balance and default nonce/code_hash.
pub fn eoa_info(balance: u64) -> AccountInfo {
    use ethrex_common::constants::EMPTY_KECCACK_HASH;
    AccountInfo {
        balance: U256::from(balance),
        nonce: 0,
        code_hash: *EMPTY_KECCACK_HASH,
    }
}
