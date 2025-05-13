use std::{collections::HashMap, str::FromStr, u64};

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, Criterion};
use ethrex::{
    cli::{import_blocks, remove_db},
    utils::set_datadir,
    DEFAULT_DATADIR,
};
use ethrex_blockchain::{
    payload::{create_payload, BuildPayloadArgs, PayloadBuildResult},
    Blockchain,
};
use ethrex_common::{
    types::{Block, Genesis, GenesisAccount, LegacyTransaction, Signable, Transaction, TxKind},
    H160, U256,
};
use ethrex_l2_sdk::{calldata::encode_calldata, get_address_from_secret_key};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::EvmEngine;
use keccak_hash::keccak;
use serde_json::json;
use secp256k1::{SecretKey, PublicKey, Secp256k1};

fn read_private_keys() -> Vec<(SecretKey, PublicKey)> {
    let file = include_str!("../../../test_data/private_keys_l1.txt");
    let secp = Secp256k1::new();
    file.lines().map(|line| {
        let line = line.trim().strip_prefix("0x").unwrap();
        let sk = SecretKey::from_str(line).unwrap();
        let pk = PublicKey::from_secret_key(&secp, &sk);
        (sk, pk)
    })
    .collect()
}

async fn setup_genesis() -> (Store, Genesis) {
    let storage_path = set_datadir(DEFAULT_DATADIR);
    remove_db(&storage_path, true);
    let genesis_file = include_bytes!("../../../test_data/genesis-l1-dev.json");
    let mut genesis: Genesis = serde_json::from_slice(genesis_file).unwrap();
    let store = Store::new(&storage_path, EngineType::Libmdbx).unwrap();
    genesis.alloc.iter_mut().for_each(|(_, account_info)| account_info.balance += u64::MAX.into());
    store.add_initial_state(genesis.clone()).await.unwrap();
    return (store, genesis);
}

async fn create_payload_block(genesis_block: &Block, store: &Store) -> Block {
    let payload_args = BuildPayloadArgs {
        parent: genesis_block.hash(),
        timestamp: genesis_block.header.timestamp,
        fee_recipient: H160::random(),
        random: genesis_block.header.prev_randao,
        withdrawals: None,
        beacon_root: genesis_block.header.parent_beacon_block_root,
        version: 3,
    };
    let block = create_payload(&payload_args, &store).unwrap();
    return block;
}


async fn fill_mempool(b: &Blockchain, accounts: Vec<SecretKey>) {
    let mut txs = vec![];
    for sk in accounts {
        for n in 0..10 {
            let mut tx = LegacyTransaction {
                nonce: n,
                gas_price: 1,
                to: TxKind::Call(H160::random()),
                value: U256::zero(),
                data: Bytes::new(),
                v: U256::one(),
                r: U256::one(),
                s: U256::one(),
                gas: 21000,
            };
            tx.sign_inplace(&sk);
            txs.push(tx)
        }
    }
    for tx in txs {
        let transaction = Transaction::LegacyTransaction(tx);
        b.add_transaction_to_pool(transaction).await.unwrap();
    }
}



pub fn build_block_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Block building");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut blockchain: Option<Blockchain> = None;
    rt.block_on(async {
        let accounts = read_private_keys();
        let (store, genesis) = setup_genesis().await;

        let mut accounts_with_balance = vec![];

        for (sk, pk) in accounts.iter() {
            let address = get_address_from_secret_key(&sk).unwrap();
            let account_info = store.get_account_info(0, address)
                            .await
                            .expect("DB error");
            let exists = account_info.is_some();
            let is_in_genesis = genesis.alloc.get(&address).is_some();
            let has_balance = account_info.map(|info| info.balance > U256::from_dec_str("1000").unwrap()).unwrap_or(false);
            if exists && has_balance && is_in_genesis {
                accounts_with_balance.push(sk.clone())
            }
        }
        blockchain = Some(Blockchain::new(EvmEngine::LEVM, store.clone()));
        fill_mempool(&blockchain.unwrap(), accounts_with_balance).await;
        // let payload_bloack = create_payload_block(&genesis.get_block(), &store).await;
    });
    group.sample_size(10);
    // group.bench_function("Build block with transfers", |b| b.iter(block_import));
    group.finish();
}

criterion_group!(runner, build_block_benchmark);
criterion_main!(runner);
