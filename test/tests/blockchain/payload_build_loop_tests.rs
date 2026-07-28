//! Regression tests for `build_payload_loop`'s post-loop final mempool
//! re-drain and its interaction with cancellation (`engine_getPayload`).
//!
//! Two properties must hold at the same time:
//!
//! - a transaction that lands in the tight window between the loop's last
//!   completed build and a `getPayload`-triggered cancellation must still be
//!   included when the held payload is empty — otherwise the builder returns
//!   an empty block while the mempool has transactions (the Hive
//!   "Invalid Missing Ancestor Syncing ReOrg" flake family);
//! - once cancelled with a non-empty payload in hand, the loop must NOT pay
//!   for one more full block build: `engine_getPayload` is blocked on it and
//!   a full rebuild of a large block can exceed the Engine API 1s deadline
//!   by itself (observed as missed proposals under sustained mempool inflow,
//!   where `tx_seq()` has always advanced and the re-drain guard never
//!   short-circuits).
//!
//! The tests drive the `build_payload_loop` future manually (its initial
//! build runs synchronously up to the first `await`) so the ordering between
//! builds, cancellation, and late-arriving transactions is fully
//! deterministic — no sleeps, no timing assumptions.

use std::{
    collections::BTreeMap,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions, BlockchainType,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, U256,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, Genesis, GenesisAccount, Transaction,
        TxKind,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;
use tokio_util::sync::CancellationToken;

const L1_CHAIN_ID: u64 = 9;

fn read_private_keys(amount: usize) -> Vec<SecretKey> {
    let file = include_str!("../../../fixtures/keys/private_keys_l1.txt");
    file.lines()
        .take(amount)
        .map(|line| {
            let line = line.trim().strip_prefix("0x").unwrap();
            SecretKey::from_str(line).unwrap()
        })
        .collect()
}

fn address_for(sk: &SecretKey) -> Address {
    Signer::Local(LocalSigner::new(*sk)).address()
}

async fn setup_store(accounts: &[Address]) -> Store {
    let genesis_file = include_bytes!("../../../fixtures/genesis/l1.json");
    let mut genesis: Genesis = serde_json::from_slice(genesis_file).unwrap();
    for address in accounts {
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
    let mut store = Store::new("store.db", EngineType::InMemory).unwrap();
    store.add_initial_state(genesis).await.unwrap();
    store
}

fn blockchain_for(store: &Store) -> Arc<Blockchain> {
    Arc::new(Blockchain::new(
        store.clone(),
        BlockchainOptions {
            r#type: BlockchainType::L1,
            perf_logs_enabled: false,
            ..Default::default()
        },
    ))
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
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    create_payload(&args, store, Bytes::new()).unwrap()
}

async fn signed_transfer(sk: &SecretKey, nonce: u64) -> Transaction {
    let signer = Signer::Local(LocalSigner::new(*sk));
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        nonce,
        value: 1_u64.into(),
        gas_limit: 250_000_u64,
        max_fee_per_gas: 10_000_000_000_u64,
        max_priority_fee_per_gas: 10_u64,
        chain_id: L1_CHAIN_ID,
        to: TxKind::Call(H160::random()),
        ..Default::default()
    });
    tx.sign_inplace(&signer).await.unwrap();
    tx
}

/// Polls the future exactly once, asserting it is still pending. For
/// `build_payload_loop` this runs the initial synchronous build and parks the
/// loop in its `select!`, with the `tx_added` waiter already registered.
macro_rules! poll_once_pending {
    ($fut:expr) => {
        std::future::poll_fn(|cx| {
            assert!(
                $fut.as_mut().poll(cx).is_pending(),
                "build loop finished before it was cancelled"
            );
            std::task::Poll::Ready(())
        })
        .await
    };
}

/// A tx that lands between the last completed build (here: the initial empty
/// build) and cancellation must be included in the returned payload. This is
/// the property the final re-drain exists for; losing it re-introduces the
/// empty-payload race on quick FCU → getPayload sequences.
#[tokio::test]
async fn cancelled_loop_still_drains_mempool_when_payload_is_empty() {
    let keys = read_private_keys(1);
    let accounts: Vec<Address> = keys.iter().map(address_for).collect();
    let store = setup_store(&accounts).await;
    let blockchain = blockchain_for(&store);

    // Mempool is empty: the initial build produces an empty payload.
    let token = CancellationToken::new();
    let mut fut = std::pin::pin!(
        blockchain
            .clone()
            .build_payload_loop(payload_block(&store), token.clone())
    );
    poll_once_pending!(fut);

    // The tx lands, then getPayload cancels before any rebuild completes.
    let tx = signed_transfer(&keys[0], 0).await;
    let tx_hash = blockchain.add_transaction_to_pool(tx).await.unwrap();
    token.cancel();

    let res = fut.await.unwrap();
    assert!(
        res.payload
            .body
            .transactions
            .iter()
            .any(|tx| tx.hash(&NativeCrypto) == tx_hash),
        "tx that landed before cancellation is missing from the payload"
    );
}

/// Once cancelled with a non-empty payload in hand, the loop must return that
/// payload as-is instead of paying for one more full build: a tx that lands
/// strictly after cancellation must NOT appear (its inclusion would prove the
/// post-cancellation re-drain ran, which is what breaches the getPayload
/// deadline under sustained mempool inflow).
#[tokio::test]
async fn cancelled_loop_returns_held_payload_without_final_rebuild() {
    let keys = read_private_keys(2);
    let accounts: Vec<Address> = keys.iter().map(address_for).collect();
    let store = setup_store(&accounts).await;
    let blockchain = blockchain_for(&store);

    // One tx in the mempool before the loop starts: the initial build holds it.
    let early_tx = signed_transfer(&keys[0], 0).await;
    let early_hash = blockchain.add_transaction_to_pool(early_tx).await.unwrap();

    let token = CancellationToken::new();
    let mut fut = std::pin::pin!(
        blockchain
            .clone()
            .build_payload_loop(payload_block(&store), token.clone())
    );
    poll_once_pending!(fut);

    // getPayload cancels; only afterwards does another tx land.
    token.cancel();
    let late_tx = signed_transfer(&keys[1], 0).await;
    let late_hash = blockchain.add_transaction_to_pool(late_tx).await.unwrap();

    let res = fut.await.unwrap();
    let hashes: Vec<_> = res
        .payload
        .body
        .transactions
        .iter()
        .map(|tx| tx.hash(&NativeCrypto))
        .collect();
    assert!(
        hashes.contains(&early_hash),
        "tx built before cancellation is missing from the payload"
    );
    assert!(
        !hashes.contains(&late_hash),
        "tx that landed after cancellation was included: the loop paid for a \
         full post-cancellation rebuild on the getPayload critical path"
    );
}

/// Manual reproduction of the `engine_getPayload` deadline breach seen on
/// glamsterdam-devnet-7: with sustained mempool inflow, every `getPayload`
/// paid for one full extra block build after cancellation. Run with:
/// `cargo test -p ethrex-test getpayload_latency -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual latency reproduction; timing-based"]
async fn getpayload_latency_under_sustained_mempool_inflow() {
    const SENDERS: usize = 8;
    const TXS_PER_SENDER: u64 = 400;

    let keys = read_private_keys(SENDERS + 1);
    let accounts: Vec<Address> = keys.iter().map(address_for).collect();
    let store = setup_store(&accounts).await;
    let blockchain = blockchain_for(&store);

    for sk in &keys[..SENDERS] {
        for nonce in 0..TXS_PER_SENDER {
            let tx = signed_transfer(sk, nonce).await;
            blockchain.add_transaction_to_pool(tx).await.unwrap();
        }
    }

    // Reference cost of one full build over this mempool.
    let start = Instant::now();
    let reference = blockchain.build_payload(payload_block(&store)).unwrap();
    let build_time = start.elapsed();
    println!(
        "one full build: {build_time:?} ({} txs)",
        reference.payload.body.transactions.len()
    );

    // FCU with attributes: the loop builds and rebuilds for the slot while a
    // feeder keeps txs flowing (spamoor-style), so `tx_seq` always advances.
    let payload_id = 1;
    blockchain
        .clone()
        .initiate_payload_build(payload_block(&store), payload_id)
        .await;
    let feeder_blockchain = blockchain.clone();
    let feeder_key = keys[SENDERS];
    let feeder = tokio::spawn(async move {
        for nonce in 0.. {
            let tx = signed_transfer(&feeder_key, nonce).await;
            if feeder_blockchain.add_transaction_to_pool(tx).await.is_err() {
                break;
            }
            // Inter-arrival below one build time, like the devnet's sustained
            // inflow: some tx always lands while a rebuild is in flight, so
            // `tx_seq()` is ahead of `last_built_seq` at cancellation time.
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    });

    // The CL requests the payload late in the slot.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let start = Instant::now();
    let res = blockchain.get_payload(payload_id).await.unwrap();
    let get_payload_time = start.elapsed();
    feeder.abort();

    println!(
        "get_payload: {get_payload_time:?} ({} txs in payload)",
        res.payload.body.transactions.len()
    );
    assert!(
        !res.payload.body.transactions.is_empty(),
        "payload must not be empty under a loaded mempool"
    );
    assert!(
        get_payload_time < build_time / 2,
        "get_payload ({get_payload_time:?}) paid for a full extra block build \
         (one build: {build_time:?}) after cancellation"
    );
}
