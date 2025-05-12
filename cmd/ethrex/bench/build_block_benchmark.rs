use std::collections::HashMap;

use criterion::{criterion_group, criterion_main, Criterion};
use ethrex::{
    cli::{import_blocks, remove_db},
    utils::set_datadir,
    DEFAULT_DATADIR,
};
use ethrex_blockchain::{
    payload::{create_payload, BuildPayloadArgs},
    Blockchain,
};
use ethrex_common::{
    types::{Block, Genesis, GenesisAccount},
    H160,
};
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::EvmEngine;
use serde_json::json;

const ERC20_ADDRESS: H160 = H160::repeat_byte(0xF);

fn setup_genesis_with_erc20_contract() -> (Block, Store) {
    let data_dir = DEFAULT_DATADIR;
    set_datadir(data_dir);
    remove_db(data_dir, true);

    let genesis_file = include_bytes!("../../../test_data/genesis-perf-ci.json");
    let mut genesis: Genesis = serde_json::from_slice(genesis_file).unwrap();
    let erc20_contract = include_str!("../../../test_data/ERC20/ERC20.bin/TestToken.bin");
    let erc20_genesis_account = GenesisAccount {
        code: erc20_contract.into(),
        storage: HashMap::new(),
        balance: u64::MAX.into(),
        nonce: 0,
    };
    genesis.alloc.insert(ERC20_ADDRESS, erc20_genesis_account);
    let store = Store::new(data_dir, EngineType::Libmdbx).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(store.add_initial_state(genesis)).unwrap();
    return (genesis, store);
}

fn build_payload(genesis_block: &Block, store: &Store) -> Block {
    let payload_args = BuildPayloadArgs {
        parent: genesis_block.hash(),
        timestamp: genesis_block.header.timestamp,
        fee_recipient: H160::random(),
        random: genesis_block.header.prev_randao,
        withdrawals: None,
        beacon_root: genesis_block.header.parent_beacon_block_root,
        version: 3,
    };
    let block = create_payload(&payload_args, store).unwrap();
    let blockchain: Blockchain = todo!();
    let result = blockchain.build_payload(block);
    todo!()
}

pub fn import_blocks_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Block building");
    group.sample_size(10);
    let evm = EvmEngine::LEVM;
    let (_, store) = setup_genesis_with_erc20_contract();
    let blockchain = Blockchain::new(evm, store);

    // group.bench_function("Block import ERC20 transfers", |b| b.iter(block_import));
    group.finish();
}

criterion_group!(runner, import_blocks_benchmark);
criterion_main!(runner);
