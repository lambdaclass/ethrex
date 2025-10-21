use std::sync::Arc;

use ethrex::{cli::remove_db, utils::default_datadir};
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, PayloadBuildResult, create_payload},
};
use ethrex_common::{
    Address, Bytes, H256,
    types::{Block, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER},
};
use ethrex_config::networks::Network;
use ethrex_storage::{EngineType, Store};

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
        let store_inner = Store::new(datadir, EngineType::RocksDB).expect("failed to create store");

        store_inner
            .add_initial_state(genesis.clone())
            .await
            .expect("failed to add genesis state to store");

        store_inner
    };

    println!("Creating blockchain instance...");

    let blockchain = Arc::new(Blockchain::new(store.clone(), BlockchainOptions::default()));

    let mut head_block_hash = genesis.get_block().hash();
    let mut head_block_timestamp = genesis.timestamp;

    for n_blocks in 1..7200 {
        println!("Testing {n_blocks} blocks batch");

        println!("Producing L1 blocks...");

        let blocks = produce_l1_blocks(
            blockchain.clone(),
            &mut store,
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
        // let tx = GenericTransaction {
        //     r#type: TxType::EIP1559,
        //     nonce: todo!(),
        //     to: todo!(),
        //     from: todo!(),
        //     gas: todo!(),
        //     value: todo!(),
        //     gas_price: todo!(),
        //     max_priority_fee_per_gas: todo!(),
        //     max_fee_per_gas: todo!(),
        //     max_fee_per_blob_gas: todo!(),
        //     access_list: todo!(),
        //     authorization_list: todo!(),
        //     blob_versioned_hashes: todo!(),
        //     blobs: todo!(),
        //     chain_id: todo!(),
        //     input: todo!(),
        // };

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
