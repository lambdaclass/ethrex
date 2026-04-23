use ethereum_types::Bloom;
use ethrex_common::{
    H256,
    constants::{DEFAULT_OMMERS_HASH, DEFAULT_REQUESTS_HASH, EMPTY_BLOCK_ACCESS_LIST_HASH},
    types::{AccountState, Block, BlockBody, BlockHeader, Genesis, INITIAL_BASE_FEE, code_hash},
};
use ethrex_crypto::NativeCrypto;
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::encode::RLPEncode;

use crate::Trie;
use crate::compute_roots::{
    compute_receipts_root, compute_storage_root, compute_transactions_root,
    compute_withdrawals_root,
};

/// Build the genesis [`Block`] from a genesis configuration.
///
/// Callers that hold a `BackendKind` should prefer
/// `StateBackend::compute_genesis_block` in `ethrex-storage` so future
/// backends are routed automatically.
pub fn genesis_block(genesis: &Genesis) -> Block {
    Block::new(genesis_header(genesis), genesis_body())
}

/// Compute the state root for a genesis configuration.
///
/// Callers that hold a `BackendKind` should prefer
/// `StateBackend::compute_genesis_root` in `ethrex-storage`.
pub fn genesis_root(genesis: &Genesis) -> H256 {
    let iter = genesis.alloc.iter().map(|(addr, account)| {
        let account_state = AccountState {
            nonce: account.nonce,
            balance: account.balance,
            storage_root: compute_storage_root(&account.storage, &NativeCrypto),
            code_hash: code_hash(&account.code, &NativeCrypto),
        };
        (keccak_hash(addr).to_vec(), account_state.encode_to_vec())
    });
    Trie::compute_hash_from_unsorted_iter(iter, &NativeCrypto)
}

fn genesis_header(genesis: &Genesis) -> BlockHeader {
    let mut blob_gas_used: Option<u64> = None;
    let mut excess_blob_gas: Option<u64> = None;

    if let Some(cancun_time) = genesis.config.cancun_time
        && cancun_time <= genesis.timestamp
    {
        blob_gas_used = Some(genesis.blob_gas_used.unwrap_or(0));
        excess_blob_gas = Some(genesis.excess_blob_gas.unwrap_or(0));
    }
    let base_fee_per_gas = genesis.base_fee_per_gas.or_else(|| {
        genesis
            .config
            .is_london_activated(0)
            .then_some(INITIAL_BASE_FEE)
    });

    let withdrawals_root = genesis
        .config
        .is_shanghai_activated(genesis.timestamp)
        .then_some(compute_withdrawals_root(&[], &NativeCrypto));

    let parent_beacon_block_root = genesis
        .config
        .is_cancun_activated(genesis.timestamp)
        .then_some(H256::zero());

    let requests_hash = genesis
        .config
        .is_prague_activated(genesis.timestamp)
        .then_some(genesis.requests_hash.unwrap_or(*DEFAULT_REQUESTS_HASH));

    let block_access_list_hash = genesis
        .config
        .is_amsterdam_activated(genesis.timestamp)
        .then_some(
            genesis
                .block_access_list_hash
                .unwrap_or(*EMPTY_BLOCK_ACCESS_LIST_HASH),
        );
    let slot_number = genesis.slot_number;

    BlockHeader {
        parent_hash: H256::zero(),
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase: genesis.coinbase,
        state_root: genesis_root(genesis),
        transactions_root: compute_transactions_root(&[], &NativeCrypto),
        receipts_root: compute_receipts_root(&[], &NativeCrypto),
        logs_bloom: Bloom::zero(),
        difficulty: genesis.difficulty,
        number: 0,
        gas_limit: genesis.gas_limit,
        gas_used: 0,
        timestamp: genesis.timestamp,
        extra_data: genesis.extra_data.clone(),
        prev_randao: genesis.mix_hash,
        nonce: genesis.nonce,
        base_fee_per_gas,
        withdrawals_root,
        blob_gas_used,
        excess_blob_gas,
        parent_beacon_block_root,
        requests_hash,
        block_access_list_hash,
        slot_number,
        ..Default::default()
    }
}

fn genesis_body() -> BlockBody {
    BlockBody {
        transactions: vec![],
        ommers: vec![],
        withdrawals: Some(vec![]),
    }
}
