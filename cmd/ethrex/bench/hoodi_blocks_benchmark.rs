use std::{cell::OnceCell, future::Future, path::Path};

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ethrex::{
    cli::{import_blocks, remove_db},
    utils::{self, set_datadir},
    DEFAULT_DATADIR,
};
use ethrex_blockchain::Blockchain;
use ethrex_common::types::{Block, Genesis};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rpc::types::block_identifier::BlockIdentifier;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::{backends::levm::LEVM, EvmEngine};
use tempdir::TempDir;


fn read_blocks() -> Vec<Block> {
    let blocks_file =
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_str().unwrap();
    utils::read_chain_file(&format!("{blocks_file}/test_data/hoodi-1-2000.rlp"))
}

async fn setup_store(blocks: &[Block]) -> Store {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let genesis_path = workspace_root
        .join("test_data")
        .join("hoodi-genesis.json");
    let data_dir = TempDir::new("benchmark").unwrap();
    let store = Store::new(
        data_dir.path().to_str().unwrap(),
        EngineType::Libmdbx).unwrap();

    let genesis: Genesis = serde_json::from_str(&std::fs::read_to_string(genesis_path).unwrap()).unwrap();
    store.add_initial_state(genesis).await.unwrap();

    let bl = Blockchain::new(EvmEngine::LEVM, store.clone());
    for block in blocks {
        bl.add_block(block).await.unwrap();
    }
    return store;
}

async fn bench_blocks(s: Store, blocks: &[Block]) {
    println!("BENCHING BLOCKS");
    let bl = Blockchain::new(EvmEngine::LEVM, s.clone());
    for b in blocks {
        bl.add_block(&b).await.unwrap();
    }
    println!("LATEST BLOCK NUMBER: {}", s.get_latest_block_number().await.unwrap());
}


pub fn hoodi_blocks_benchmark(c: &mut Criterion) {
    println!("BENCHMARK STARTS");
    let blocks = read_blocks();
    let (pre_blocks, blocks) = blocks.split_at(1000);
    let store = {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(setup_store(pre_blocks))
    };
    println!("SETUP FINISHED");
    c.bench_function("benchmarking hoodi blocks", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async {
                black_box(bench_blocks(store.clone(), blocks).await)
            })
    });
}

criterion_group!(runner, hoodi_blocks_benchmark);
criterion_main!(runner);
