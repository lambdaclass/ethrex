//! Tests for the `BodyValidation` gating in the block import pipeline: the
//! default path must keep rejecting bodies whose transactions root does not
//! match the header, while the opt-out for pre-validated bodies (engine
//! newPayload) must skip only the root recomputation and keep the cheap
//! structural checks.
use std::collections::BTreeMap;

use ethrex_blockchain::error::{ChainError, InvalidBlockError};
use ethrex_blockchain::{Blockchain, BodyValidation};
use ethrex_common::constants::DEFAULT_OMMERS_HASH;
use ethrex_common::types::{
    Block, BlockBody, BlockHeader, ChainConfig, ELASTICITY_MULTIPLIER, Genesis,
    InvalidBlockBodyError, Receipt, calculate_base_fee_per_gas, compute_receipts_root,
    compute_transactions_root,
};
use ethrex_common::{Address, Bloom, Bytes, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{EngineType, Store};

/// Minimal post-merge (pre-Shanghai) genesis with an in-memory store.
async fn setup() -> (Store, BlockHeader) {
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
    let mut store = Store::new("", EngineType::InMemory).expect("open in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("initialize genesis");
    (store, parent)
}

/// Builds a valid empty child block of `parent` (no transactions, no withdrawals).
fn empty_child_block(parent: &BlockHeader) -> Block {
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
        parent_hash: parent.hash(),
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase: Address::zero(),
        // No state changes in an empty pre-Shanghai block.
        state_root: parent.state_root,
        transactions_root: compute_transactions_root(&body.transactions, &NativeCrypto),
        receipts_root: compute_receipts_root(&receipts, &NativeCrypto),
        logs_bloom: Bloom::zero(),
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

    Block::new(header, body)
}

#[tokio::test]
async fn import_pipeline_rejects_tampered_transactions_root() {
    let (store, parent) = setup().await;
    let mut block = empty_child_block(&parent);
    // Tampered: the body is empty, so the correct transactions root is the
    // empty-trie hash, not zero.
    block.header.transactions_root = H256::zero();

    let blockchain = Blockchain::default_with_store(store);
    let err = blockchain
        .add_block_pipeline(block, None)
        .expect_err("a body that does not match the header transactions root must be rejected");
    assert!(
        matches!(
            err,
            ChainError::InvalidBlock(InvalidBlockError::InvalidBody(
                InvalidBlockBodyError::TransactionsRootNotMatch
            ))
        ),
        "expected TransactionsRootNotMatch, got: {err:?}"
    );
}

#[tokio::test]
async fn prevalidated_import_still_rejects_nonempty_ommers() {
    let (store, parent) = setup().await;
    let mut block = empty_child_block(&parent);
    // The structural check must run even when the caller vouches for the roots.
    block.body.ommers.push(parent.clone());

    let blockchain = Blockchain::default_with_store(store);
    let err = blockchain
        .add_block_pipeline_with_body_validation(block, None, BodyValidation::AlreadyValidated)
        .expect_err("a post-merge body with ommers must be rejected");
    assert!(
        matches!(
            err,
            ChainError::InvalidBlock(InvalidBlockError::InvalidBody(
                InvalidBlockBodyError::OmmersIsNotEmpty
            ))
        ),
        "expected OmmersIsNotEmpty, got: {err:?}"
    );
}

#[tokio::test]
async fn prevalidated_import_accepts_valid_block() {
    let (store, parent) = setup().await;
    let block = empty_child_block(&parent);

    let blockchain = Blockchain::default_with_store(store);
    blockchain
        .add_block_pipeline_with_body_validation(block, None, BodyValidation::AlreadyValidated)
        .expect("a valid block must import through the pre-validated path");
}
