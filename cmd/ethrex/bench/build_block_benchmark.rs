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
    types::{Block, Genesis, GenesisAccount, LegacyTransaction, Signable, Transaction, TxKind, EIP1559Transaction},
    Address, H160, U256,
};
use ethrex_l2_sdk::{calldata::encode_calldata, get_address_from_secret_key};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::EvmEngine;
use keccak_hash::keccak;
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use serde_json::json;

fn address_from_pub_key(public_key: PublicKey) -> H160 {
    let bytes = public_key.serialize_uncompressed();
    let hash = keccak(&bytes[1..]);
    let address_bytes: [u8; 20] = hash.as_ref().get(12..32).unwrap().try_into().unwrap();

    Address::from(address_bytes)
}
fn read_private_keys() -> Vec<SecretKey> {
    let file = include_str!("../../../test_data/private_keys_l1.txt");
    file.lines()
        .map(|line| {
            let line = line.trim().strip_prefix("0x").unwrap();
            let sk = SecretKey::from_str(line).unwrap();
            sk
        })
        .collect()
}

fn recover_address_for_sk(sk: &SecretKey) -> Address {
    let mut tx = Transaction::LegacyTransaction(LegacyTransaction {
        nonce: 0,
        gas_price: 1,
        to: TxKind::Call(H160::random()),
        value: U256::zero(),
        data: Bytes::new(),
        v: U256::one(),
        r: U256::one(),
        s: U256::one(),
        gas: 21000,
    });
    tx.sign_inplace(sk);
    tx.sender()
}

async fn setup_genesis(accounts: &Vec<Address>) -> (Store, Genesis) {
    let storage_path = set_datadir(DEFAULT_DATADIR);
    remove_db(&storage_path, true);
    let genesis_file = include_bytes!("../../../test_data/genesis-l1-dev.json");
    let mut genesis: Genesis = serde_json::from_slice(genesis_file).unwrap();
    let store = Store::new(&storage_path, EngineType::Libmdbx).unwrap();
    for address in accounts {
        let account_info = GenesisAccount {
            code: Bytes::new(),
            storage: HashMap::new(),
            balance: u64::MAX.into(),
            nonce: 0,
        };
        genesis.alloc.insert(address.clone(), account_info);
    }
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
            let mut tx =
                Transaction::EIP1559Transaction(EIP1559Transaction {
                    value: 1_u64.into(),
                    gas_limit: (24000000_u64).into(),
                    chain_id: 9,
                    ..Default::default()
                });
            tx.sign_inplace(&sk);
            txs.push(tx);
        }
    }
    for tx in txs {
        b.add_transaction_to_pool(tx).await.unwrap();
    }
}

pub fn build_block_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Block building");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut blockchain: Option<Blockchain> = None;
    rt.block_on(async {
        let accounts = read_private_keys();
        let addresses = accounts
            .clone()
            .into_iter()
            .map(|sk| recover_address_for_sk(&sk))
            .collect();
        let (store, _) = setup_genesis(&addresses).await;
        blockchain = Some(Blockchain::new(EvmEngine::LEVM, store.clone()));
        fill_mempool(&blockchain.unwrap(), accounts).await;
    });
    group.sample_size(10);
    // group.bench_function("Build block with transfers", |b| b.iter(block_import));
    group.finish();
}

criterion_group!(runner, build_block_benchmark);
criterion_main!(runner);
