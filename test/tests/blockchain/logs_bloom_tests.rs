//! Regression test for the `header-logs-bloom-skip` finding: block import must
//! reject a block whose header `logs_bloom` does not match the aggregate bloom of
//! the executed receipts. Other clients (geth/reth) validate the bloom, so
//! accepting a mismatched one means ethrex imports a block the network rejects.
use std::collections::BTreeMap;

use ethrex_blockchain::Blockchain;
use ethrex_blockchain::error::{ChainError, InvalidBlockError};
use ethrex_common::constants::DEFAULT_OMMERS_HASH;
use ethrex_common::types::{
    Block, BlockBody, BlockHeader, ChainConfig, ELASTICITY_MULTIPLIER, Genesis, Receipt,
    calculate_base_fee_per_gas, compute_receipts_root, compute_transactions_root,
};
use ethrex_common::{Address, Bloom, Bytes, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{EngineType, Store};

#[tokio::test]
async fn rejects_block_with_mismatched_logs_bloom() {
    // Minimal post-merge (pre-Shanghai) chain: an empty block has no logs, so its
    // correct logs_bloom is all zeros.
    let genesis = Genesis {
        config: ChainConfig {
            chain_id: 9,
            homestead_block: Some(0),
            eip150_block: Some(0),
            eip155_block: Some(0),
            eip158_block: Some(0),
            byzantium_block: Some(0),
            constantinople_block: Some(0),
            petersburg_block: Some(0),
            istanbul_block: Some(0),
            berlin_block: Some(0),
            london_block: Some(0),
            merge_netsplit_block: Some(0),
            terminal_total_difficulty: Some(0),
            terminal_total_difficulty_passed: true,
            ..Default::default()
        },
        alloc: BTreeMap::new(),
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(0),
        ..Default::default()
    };

    let parent = genesis.get_block().header.clone();
    let parent_hash = parent.hash();
    let mut store = Store::new("", EngineType::InMemory).expect("open in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("initialize genesis");

    // Empty block: no transactions, no receipts -> the correct logs_bloom is zero.
    let body = BlockBody {
        transactions: Vec::new(),
        ommers: Vec::new(),
        withdrawals: None,
    };
    let receipts: [Receipt; 0] = [];

    let base_fee_per_gas = calculate_base_fee_per_gas(
        parent.gas_limit,
        parent.gas_limit,
        parent.gas_used,
        parent.base_fee_per_gas.unwrap_or_default(),
        ELASTICITY_MULTIPLIER,
    )
    .expect("calculate child base fee");

    let header = BlockHeader {
        parent_hash,
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase: Address::zero(),
        // No state changes in an empty pre-Shanghai block.
        state_root: parent.state_root,
        transactions_root: compute_transactions_root(&body.transactions, &NativeCrypto),
        receipts_root: compute_receipts_root(&receipts, &NativeCrypto),
        // Tampered: the block has no logs, so the correct bloom is zero, but the
        // header advertises a non-zero bloom. Other clients reject this.
        logs_bloom: Bloom::repeat_byte(0xff),
        difficulty: U256::zero(),
        number: parent.number + 1,
        gas_limit: parent.gas_limit,
        gas_used: 0,
        timestamp: parent.timestamp + 1,
        extra_data: Bytes::new(),
        prev_randao: H256::zero(),
        nonce: 0,
        base_fee_per_gas: Some(base_fee_per_gas),
        withdrawals_root: None,
        blob_gas_used: None,
        excess_blob_gas: None,
        parent_beacon_block_root: None,
        requests_hash: None,
        block_access_list_hash: None,
        slot_number: None,
        ..Default::default()
    };

    let block = Block::new(header, body);
    let blockchain = Blockchain::default_with_store(store);

    // Assert the *specific* rejection reason. The block is otherwise valid (correct
    // receipts root, state root, etc.), so a bare `is_err()` could mask the bloom check
    // silently rotting behind some other failure.
    let err = blockchain.add_block(block).expect_err(
        "a block whose header logs_bloom does not match the executed receipts must be rejected",
    );
    assert!(
        matches!(
            err,
            ChainError::InvalidBlock(InvalidBlockError::LogsBloomMismatch)
        ),
        "expected LogsBloomMismatch, got: {err:?}"
    );
}
