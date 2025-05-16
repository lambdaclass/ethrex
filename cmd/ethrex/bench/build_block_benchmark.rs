use std::{collections::HashMap, str::FromStr};

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, Criterion};
use ethrex::{cli::remove_db, utils::set_datadir, DEFAULT_DATADIR};
use ethrex_blockchain::{
    payload::{create_payload, BuildPayloadArgs, PayloadBuildResult},
    Blockchain,
};
use ethrex_common::{
    types::{
        payload::PayloadBundle, Block, EIP1559Transaction, Genesis, GenesisAccount,
        LegacyTransaction, Signable, Transaction, TxKind,
    },
    Address, H160, U256,
};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::EvmEngine;
use secp256k1::SecretKey;

fn read_private_keys() -> Vec<SecretKey> {
    let file = include_str!("../../../test_data/private_keys_l1.txt");
    file.lines()
        .map(|line| {
            let line = line.trim().strip_prefix("0x").unwrap();
            SecretKey::from_str(line).unwrap()
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
        genesis.alloc.insert(*address, account_info);
    }
    store.add_initial_state(genesis.clone()).await.unwrap();
    (store, genesis)
}

async fn create_payload_block(genesis_block: &Block, store: &Store) -> (Block, u64) {
    let payload_args = BuildPayloadArgs {
        parent: genesis_block.hash(),
        timestamp: genesis_block.header.timestamp + 1,
        fee_recipient: H160::random(),
        random: genesis_block.header.prev_randao,
        withdrawals: None,
        beacon_root: genesis_block.header.parent_beacon_block_root,
        version: 3,
        elasticity_multiplier: 1,
    };
    let id = payload_args.id();
    let block = create_payload(&payload_args, store).unwrap();
    (block, id)
}

async fn fill_mempool(b: &Blockchain, accounts: Vec<SecretKey>) {
    let mut txs = vec![];
    for sk in accounts {
        for n in 0..1000 {
            let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
                nonce: n,
                value: 1_u64.into(),
                gas_limit: 250000_u64,
                max_fee_per_gas: u64::MAX,
                max_priority_fee_per_gas: 10_u64,
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

pub async fn bench_payload(input: &(&mut Blockchain, Block, &Store)) {
    let (b, genesis_block, store) = input;
    // 1. engine_forkChoiceUpdated is called, which ends up calling fork_choice::build_payload,
    // which finally calls payload::create_payload(), this mimics this step without
    // the RPC handling. The payload is created and the id stored.
    let (payload_block, payload_id) = create_payload_block(genesis_block, store).await;
    store
        .add_payload(payload_id, payload_block.clone())
        .await
        .unwrap();
    // 2. engine_getPayload is called, this code path ends up calling Store::get_payload(id),
    // so we also mimic that here without the RPC part.
    // We also need to updated the payload to set it as completed.
    // Blockchain::build_payload eventaully calls to 'fill_transactions'
    // which should take transactions from the previously filled mempool.
    let payload = store.get_payload(payload_id).await.unwrap().unwrap();
    let (blobs_bundle, requests, block_value, block) = {
        let PayloadBuildResult {
            blobs_bundle,
            block_value,
            requests,
            payload,
            ..
        } = b.build_payload(payload.block.clone()).await.unwrap();
        (blobs_bundle, requests, block_value, payload)
    };
    let new_payload = PayloadBundle {
        block: block.clone(),
        block_value,
        blobs_bundle,
        requests,
        completed: true,
    };
    store.update_payload(payload_id, new_payload).await.unwrap();
    // 3. engine_newPayload is called, this eventually calls Blockchain::add_block
    // which takes transactions from the mempool and fills the block with them.
    b.add_block(&block).await.unwrap();
    // EXTRA: Sanity check to not benchmark n empty block.
    let hash = &block.hash();
    assert!(!store
        .get_block_body_by_hash(*hash)
        .await
        .unwrap()
        .unwrap()
        .transactions
        .is_empty());
}

pub fn build_block_benchmark(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mut blockchain, genesis_block, store) = rt.block_on(async {
        let accounts = read_private_keys();
        let addresses = accounts
            .clone()
            .into_iter()
            .map(|sk| recover_address_for_sk(&sk))
            .collect();

        let (store_with_genesis, gen) = setup_genesis(&addresses).await;
        let block_chain = Blockchain::new(EvmEngine::LEVM, store_with_genesis.clone());
        fill_mempool(&block_chain, accounts).await;

        (block_chain, gen.get_block(), store_with_genesis)
    });
    let input = (&mut blockchain, genesis_block, &store);
    c.bench_function("block payload building bench", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| bench_payload(&input))
    });
}

criterion_group!(runner, build_block_benchmark);
criterion_main!(runner);
