//! TEMPORARY seal-time measurement for large state diffs. Not for commit.
//! `cargo test --release -p ethrex-test --test ethrex_tests seal_diff -- --ignored --nocapture --test-threads 1`

use std::{collections::BTreeMap, sync::Arc, time::Instant};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions, BlockchainType,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, U256,
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
const PREFILL_ACCOUNTS: u64 = 1_000_000;
const HOT_SLOTS: u64 = 1_000_000;
const HOT_SLOTS_PER_TX: u64 = 3_000;
const SPREAD_SLOTS_PER_CONTRACT: u64 = 2_000;
const SPREAD_SLOTS_PER_TX: u64 = 100;
const SLOT_BASE: u64 = 0xA000_0000_0000;
const HOT_CONTRACT: Address = H160([0xcc; 20]);

// calldata = start ‖ count ‖ value (32 bytes each); sstore(start + k, value) for k < count.
//   00 PUSH1 20  CALLDATALOAD        ; count
//   03 PUSH1 00  CALLDATALOAD        ; start      [i, remaining]
//   06 JUMPDEST  DUP2 ISZERO PUSH1 1d JUMPI
//   0c PUSH1 40  CALLDATALOAD DUP2 SSTORE        ; sstore(i, value)
//   11 PUSH1 01  ADD SWAP1 PUSH1 01 SWAP1 SUB SWAP1 ; [i+1, remaining-1]
//   1a PUSH1 06  JUMP
//   1d JUMPDEST  STOP
const LOOP_CODE: [u8; 31] = [
    0x60, 0x20, 0x35, 0x60, 0x00, 0x35, 0x5b, 0x81, 0x15, 0x60, 0x1d, 0x57, 0x60, 0x40, 0x35,
    0x81, 0x55, 0x60, 0x01, 0x01, 0x90, 0x60, 0x01, 0x90, 0x03, 0x90, 0x60, 0x06, 0x56, 0x5b,
    0x00,
];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Workload {
    /// Each tx overwrites 3000 existing slots of one contract holding 1M slots.
    UpdateHot,
    /// Same, but writes zero, deleting the slots.
    ClearHot,
    /// Each tx overwrites 100 existing slots of its own contract (2000 slots each).
    UpdateSpread,
}

fn spread_contract(j: u64) -> Address {
    H160::from_low_u64_be(0xC000_0000_0000 + j)
}

fn contract_account(slots: u64) -> GenesisAccount {
    GenesisAccount {
        code: Bytes::from(LOOP_CODE.to_vec()),
        storage: (0..slots)
            .map(|i| (U256::from(SLOT_BASE + i), U256::one()))
            .collect(),
        balance: U256::zero(),
        nonce: 1,
    }
}

async fn setup(senders: &[Address], workload: Workload, txs: u64) -> Store {
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
    for i in 0..PREFILL_ACCOUNTS {
        genesis.alloc.insert(
            H160::from_low_u64_be(0xA000_0000_0000 + i),
            GenesisAccount {
                code: Bytes::new(),
                storage: BTreeMap::new(),
                balance: U256::one(),
                nonce: 0,
            },
        );
    }
    if workload == Workload::UpdateSpread {
        for j in 0..txs {
            genesis
                .alloc
                .insert(spread_contract(j), contract_account(SPREAD_SLOTS_PER_CONTRACT));
        }
    } else {
        genesis.alloc.insert(HOT_CONTRACT, contract_account(HOT_SLOTS));
    }
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

fn word(v: u64) -> [u8; 32] {
    U256::from(v).to_big_endian()
}

async fn tx_for(workload: Workload, sk: &SecretKey, nonce: u64, idx: u64) -> Transaction {
    let (to, start, count, value, gas_limit) = match workload {
        Workload::UpdateHot => (
            HOT_CONTRACT,
            SLOT_BASE + idx * HOT_SLOTS_PER_TX,
            HOT_SLOTS_PER_TX,
            2,
            16_000_000,
        ),
        Workload::ClearHot => (
            HOT_CONTRACT,
            SLOT_BASE + idx * HOT_SLOTS_PER_TX,
            HOT_SLOTS_PER_TX,
            0,
            16_000_000,
        ),
        Workload::UpdateSpread => (
            spread_contract(idx),
            SLOT_BASE,
            SPREAD_SLOTS_PER_TX,
            2,
            600_000,
        ),
    };
    let mut data = Vec::with_capacity(96);
    data.extend_from_slice(&word(start));
    data.extend_from_slice(&word(count));
    data.extend_from_slice(&word(value));
    let signer = Signer::Local(LocalSigner::new(*sk));
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        nonce,
        value: U256::zero(),
        gas_limit,
        max_fee_per_gas: 10_000_000_000_u64,
        max_priority_fee_per_gas: 10_u64,
        chain_id: CHAIN_ID,
        to: TxKind::Call(to),
        data: Bytes::from(data),
        ..Default::default()
    });
    tx.sign_inplace(&signer).await.unwrap();
    tx
}

async fn measure(workload: Workload, txs: u64) {
    let keys = read_private_keys(SENDERS);
    let senders: Vec<Address> = keys.iter().map(address_for).collect();
    let t = Instant::now();
    let store = setup(&senders, workload, txs).await;
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
        let sk = &keys[i as usize % SENDERS];
        let tx = tx_for(workload, sk, i / SENDERS as u64, i).await;
        blockchain.add_transaction_to_pool(tx).await.unwrap();
    }
    let slots = txs
        * if workload == Workload::UpdateSpread {
            SPREAD_SLOTS_PER_TX
        } else {
            HOT_SLOTS_PER_TX
        };

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
        assert_eq!(unsealed.transactions().len() as u64, txs, "not every tx was included");
        let gas = unsealed.gas_used();
        eprintln!("RUN {workload:?} txs={txs} slots={slots} rep={rep} gas={gas}");
        let t = Instant::now();
        let _sealed = blockchain.seal_payload(unsealed).unwrap();
        let seal = t.elapsed();
        builds.push(build);
        seals.push(seal);
    }
    builds.sort();
    seals.sort();
    eprintln!(
        "RESULT workload={workload:?} txs={txs} slots={slots} setup={setup_time:?} \
         unsealed_build_median={:?} seal_min={:?} seal_median={:?}",
        builds[REPS / 2], seals[0], seals[REPS / 2]
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual measurement"]
async fn seal_diff() {
    // About 60M, 150M and 300M gas each.
    for txs in [4, 10, 20] {
        measure(Workload::UpdateHot, txs).await;
    }
    for txs in [4, 10, 20] {
        measure(Workload::ClearHot, txs).await;
    }
    for txs in [114, 285, 570] {
        measure(Workload::UpdateSpread, txs).await;
    }
}
