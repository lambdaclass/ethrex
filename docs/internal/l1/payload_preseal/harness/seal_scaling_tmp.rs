//! TEMPORARY seal-time scaling measurement. Not for commit.
//! `cargo test --release -p ethrex-test --test ethrex_tests seal_scaling -- --ignored --nocapture --test-threads 1`

use std::{collections::BTreeMap, sync::Arc, time::Instant};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions, BlockchainType,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{Block, EIP1559Transaction, Genesis, GenesisAccount, Transaction, TxKind},
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;
use tokio_util::sync::CancellationToken;

use super::payload_build_loop_tests::{address_for, read_private_keys};

const CHAIN_ID: u64 = 9;
const GAS_LIMIT: u64 = 400_000_000;
const SENDERS: usize = 64;
const REPS: usize = 3;
// PUSH1 0 CALLDATALOAD DUP1 SSTORE STOP: stores word w at slot w.
const SSTORE_CODE: [u8; 6] = [0x60, 0x00, 0x35, 0x80, 0x55, 0x00];
const CONTRACT: Address = H160([0xcc; 20]);

#[derive(Clone, Copy, Debug)]
enum Workload {
    /// ETH transfers, each to a new account.
    FreshRecipients,
    /// ETH transfers to 4 fixed accounts.
    FixedRecipients,
    /// Each tx writes one new slot in one contract.
    FreshSlots,
}

async fn setup(senders: &[Address], prefill: usize) -> Store {
    let genesis_file = include_bytes!("../../../fixtures/genesis/l1.json");
    let mut genesis: Genesis = serde_json::from_slice(genesis_file).unwrap();
    genesis.gas_limit = GAS_LIMIT;
    for address in senders {
        genesis.alloc.insert(
            *address,
            GenesisAccount {
                code: Bytes::new(),
                storage: BTreeMap::new(),
                balance: U256::MAX,
                nonce: 0,
            },
        );
    }
    let mut contract_storage = BTreeMap::new();
    for i in 0..prefill as u64 {
        genesis.alloc.insert(
            H160::from_low_u64_be(0xA000_0000_0000 + i),
            GenesisAccount {
                code: Bytes::new(),
                storage: BTreeMap::new(),
                balance: U256::one(),
                nonce: 0,
            },
        );
        contract_storage.insert(U256::from(0xA000_0000_0000_u64 + i), U256::one());
    }
    genesis.alloc.insert(
        CONTRACT,
        GenesisAccount {
            code: Bytes::from(SSTORE_CODE.to_vec()),
            storage: contract_storage,
            balance: U256::zero(),
            nonce: 1,
        },
    );
    let mut store = Store::new("store.db", EngineType::InMemory).unwrap();
    store.add_initial_state(genesis).await.unwrap();
    store
}

fn payload_block(store: &Store) -> Block {
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: H160::random(),
        random: genesis_header.prev_randao,
        withdrawals: None,
        beacon_root: genesis_header.parent_beacon_block_root,
        slot_number: None,
        version: 3,
        elasticity_multiplier: 1,
        gas_ceil: GAS_LIMIT,
    };
    create_payload(&args, store, Bytes::new()).unwrap()
}

async fn tx_for(workload: Workload, sk: &SecretKey, nonce: u64, idx: u64) -> Transaction {
    let (to, data, gas_limit) = match workload {
        Workload::FreshRecipients => (
            H160::from_low_u64_be(0xF000_0000_0000 + idx),
            Bytes::new(),
            21_000,
        ),
        Workload::FixedRecipients => (
            H160::from_low_u64_be(0xE000_0000_0000 + idx % 4),
            Bytes::new(),
            21_000,
        ),
        Workload::FreshSlots => (
            CONTRACT,
            Bytes::from(H256::from_low_u64_be(idx + 1).0.to_vec()),
            100_000,
        ),
    };
    let signer = Signer::Local(LocalSigner::new(*sk));
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        nonce,
        value: 1_u64.into(),
        gas_limit,
        max_fee_per_gas: 10_000_000_000_u64,
        max_priority_fee_per_gas: 10_u64,
        chain_id: CHAIN_ID,
        to: TxKind::Call(to),
        data,
        ..Default::default()
    });
    tx.sign_inplace(&signer).await.unwrap();
    tx
}

async fn measure(workload: Workload, txs: usize, prefill: usize) {
    let keys = read_private_keys(SENDERS);
    let senders: Vec<Address> = keys.iter().map(address_for).collect();
    let t = Instant::now();
    let store = setup(&senders, prefill).await;
    let setup_time = t.elapsed();
    let blockchain = Arc::new(Blockchain::new(
        store.clone(),
        BlockchainOptions {
            r#type: BlockchainType::L1,
            perf_logs_enabled: false,
            max_mempool_size: 100_000,
            ..Default::default()
        },
    ));
    for i in 0..txs {
        let tx = tx_for(workload, &keys[i % SENDERS], (i / SENDERS) as u64, i as u64).await;
        blockchain.add_transaction_to_pool(tx).await.unwrap();
    }

    let mut builds = Vec::new();
    let mut seals = Vec::new();
    for rep in 0..REPS {
        let token = CancellationToken::new();
        let mut fut = std::pin::pin!(
            blockchain
                .clone()
                .build_payload_loop(payload_block(&store), token.clone())
        );
        let t = Instant::now();
        std::future::poll_fn(|cx| {
            assert!(fut.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        let build = t.elapsed();
        token.cancel();
        let unsealed = fut.await.unwrap();
        assert_eq!(unsealed.transactions().len(), txs, "not every tx was included");
        let gas = unsealed.gas_used();
        eprintln!("RUN {workload:?} txs={txs} prefill={prefill} rep={rep} gas={gas}");
        let t = Instant::now();
        let _sealed = blockchain.seal_payload(unsealed).unwrap();
        let seal = t.elapsed();
        builds.push(build);
        seals.push(seal);
    }
    builds.sort();
    seals.sort();
    eprintln!(
        "RESULT workload={workload:?} txs={txs} prefill={prefill} setup={setup_time:?} \
         unsealed_build_median={:?} seal_min={:?} seal_median={:?}",
        builds[REPS / 2], seals[0], seals[REPS / 2]
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual measurement"]
async fn seal_scaling() {
    for txs in [500, 1000, 2000, 4000, 8000, 16000] {
        measure(Workload::FreshRecipients, txs, 0).await;
    }
    for txs in [1000, 4000, 16000] {
        measure(Workload::FixedRecipients, txs, 0).await;
    }
    for txs in [1000, 2000, 4000, 8000] {
        measure(Workload::FreshSlots, txs, 0).await;
    }
    for txs in [1000, 4000, 16000] {
        measure(Workload::FreshRecipients, txs, 1_000_000).await;
    }
    for txs in [1000, 4000, 8000] {
        measure(Workload::FreshSlots, txs, 1_000_000).await;
    }
}
