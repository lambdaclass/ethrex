use std::sync::Arc;

use ethrex::{cli::remove_db, utils::default_datadir};
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, PayloadBuildResult, create_payload},
    validate_block,
};
use ethrex_common::{
    Address, Bytes, H256,
    types::{Block, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER},
};
use ethrex_config::networks::Network;
use ethrex_l2::sequencer::block_producer::build_payload;
use ethrex_storage::{EngineType, Store};
use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};
use ethrex_vm::BlockExecutionResult;

#[tokio::main]
async fn main() {
    let datadir = default_datadir();

    println!("Removing existing database...");

    remove_db(&datadir, true);

    let network = Network::LocalDevnet;

    let genesis = network
        .get_genesis()
        .expect("failed to get genesis from local devnet");

    println!("Creating store and adding genesis state...");

    let mut store = {
        let store_inner =
            Store::new(&datadir, EngineType::RocksDB).expect("failed to create store");

        store_inner
            .add_initial_state(genesis.clone())
            .await
            .expect("failed to add genesis state to store");

        store_inner
    };

    let rollup_store = {
        let rollup_store = StoreRollup::new(&datadir, EngineTypeRollup::InMemory)
            .expect("failed to create StoreRollup");
        rollup_store
            .init()
            .await
            .expect("failed to init rollup store");
        rollup_store
    };

    println!("Creating blockchain instance...");

    let blockchain = Arc::new(Blockchain::new(store.clone(), BlockchainOptions::default()));

    let mut head_block_hash = genesis.get_block().hash();
    let mut head_block_timestamp = genesis.timestamp;

    for n_blocks in 1..7200 {
        println!("Testing {n_blocks} blocks batch");

        println!("Producing blocks...");

        // let blocks = produce_l1_blocks(
        //     blockchain.clone(),
        //     &mut store,
        //     head_block_hash,
        //     head_block_timestamp + 12,
        //     n_blocks,
        // )
        // .await;

        let blocks = produce_l2_blocks(
            blockchain.clone(),
            &mut store,
            &rollup_store,
            head_block_hash,
            head_block_timestamp + 12,
            n_blocks,
        )
        .await;

        head_block_hash = blocks.last().expect("no blocks produced").hash();

        head_block_timestamp = blocks.last().expect("no blocks produced").header.timestamp;

        println!("Generating witnesses for produced blocks...");

        blockchain
            .generate_witness_for_blocks(&blocks)
            .await
            .expect("failed to generate witness for blocks");

        println!("Done!");
    }
}

pub async fn produce_l1_blocks(
    blockchain: Arc<Blockchain>,
    store: &mut Store,
    head_block_hash: H256,
    initial_timestamp: u64,
    n_blocks: u64,
) -> Vec<Block> {
    let mut blocks = Vec::new();

    let mut current_parent_hash = head_block_hash;

    let mut current_timestamp = initial_timestamp;

    for _ in 0..n_blocks {
        let block = produce_l1_block(
            blockchain.clone(),
            store,
            current_parent_hash,
            current_timestamp,
        )
        .await;

        current_parent_hash = block.hash();

        current_timestamp += 12; // Assuming an average block time of 12 seconds

        blocks.push(block);
    }

    blocks
}

pub async fn produce_l1_block(
    blockchain: Arc<Blockchain>,
    store: &mut Store,
    head_block_hash: H256,
    timestamp: u64,
) -> Block {
    let build_payload_args = BuildPayloadArgs {
        parent: head_block_hash,
        timestamp,
        fee_recipient: Address::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        version: 3,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };

    let payload_id = build_payload_args
        .id()
        .expect("failed to compute payload ID");

    let payload =
        create_payload(&build_payload_args, store, Bytes::new()).expect("failed to create payload");

    blockchain
        .clone()
        .initiate_payload_build(payload, payload_id)
        .await;

    let PayloadBuildResult { payload: block, .. } = blockchain
        .get_payload(payload_id)
        .await
        .expect("failed to get payload");

    blockchain
        .add_block(block.clone())
        .await
        .expect("failed to add block");

    let new_block_hash = block.hash();

    apply_fork_choice(store, new_block_hash, new_block_hash, new_block_hash)
        .await
        .expect("failed to apply fork choice");

    block
}

pub async fn produce_l2_blocks(
    blockchain: Arc<Blockchain>,
    store: &mut Store,
    rollup_store: &StoreRollup,
    head_block_hash: H256,
    initial_timestamp: u64,
    n_blocks: u64,
) -> Vec<Block> {
    let mut blocks = Vec::new();

    let mut current_parent_hash = head_block_hash;

    let mut current_timestamp = initial_timestamp;

    let mut last_privilege_nonce = None;

    for _ in 0..n_blocks {
        let block = produce_l2_block(
            blockchain.clone(),
            store,
            rollup_store,
            current_parent_hash,
            current_timestamp,
            &mut last_privilege_nonce,
        )
        .await;

        current_parent_hash = block.hash();

        current_timestamp += 12; // Assuming an average block time of 12 seconds

        blocks.push(block);
    }

    blocks
}

pub async fn produce_l2_block(
    blockchain: Arc<Blockchain>,
    store: &mut Store,
    rollup_store: &StoreRollup,
    head_block_hash: H256,
    timestamp: u64,
    last_privilege_nonce: &mut Option<u64>,
) -> Block {
    let build_payload_args = BuildPayloadArgs {
        parent: head_block_hash,
        timestamp,
        fee_recipient: Address::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        version: 3,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };

    let payload =
        create_payload(&build_payload_args, store, Bytes::new()).expect("failed to create payload");

    let payload_build_result = build_payload(
        blockchain.clone(),
        payload,
        store,
        last_privilege_nonce,
        DEFAULT_BUILDER_GAS_CEIL,
    )
    .await
    .expect("failed to build payload");

    let new_block = payload_build_result.payload;

    let chain_config = store
        .get_chain_config()
        .expect("failed to get chain config");

    validate_block(
        &new_block,
        &store
            .get_block_header_by_hash(new_block.header.parent_hash)
            .expect("failed to get parent block header")
            .expect("parent block not found"),
        &chain_config,
        build_payload_args.elasticity_multiplier,
    )
    .expect("failed to validate block");

    let account_updates = payload_build_result.account_updates;

    let execution_result = BlockExecutionResult {
        receipts: payload_build_result.receipts,
        requests: Vec::new(),
    };

    let account_updates_list = store
        .apply_account_updates_batch(new_block.header.parent_hash, &account_updates)
        .await
        .expect("failed to apply account updates")
        .expect("no account updates returned");

    blockchain
        .store_block(new_block.clone(), account_updates_list, execution_result)
        .await
        .expect("failed to store block");

    rollup_store
        .store_account_updates_by_block_number(new_block.header.number, account_updates)
        .await
        .expect("failed to store account updates in rollup store");

    let new_block_hash = new_block.hash();

    apply_fork_choice(store, new_block_hash, new_block_hash, new_block_hash)
        .await
        .expect("failed to apply fork choice");

    new_block
}
