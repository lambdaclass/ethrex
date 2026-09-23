//! The parallel Amsterdam path stops executing a block once the transactions it has
//! finished are over the block gas limit.
//!
//! Ordered gas admission can only run after every transaction has executed, so without
//! that stop an over-limit block is executed in full before it is rejected. Here each
//! transaction burns its whole gas limit in a loop and emits nothing, so nothing but
//! the gas spent bounds the work: the block holds twenty times what its gas limit
//! allows, yet is small enough to pass the pre-execution minimum-work check.

use std::{fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER, Fork, Genesis,
        GenesisAccount, Transaction, TxKind, block_access_list::BlockAccessList,
        compute_transactions_root,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::check_minimum_block_work;
use secp256k1::SecretKey;

/// `JUMPDEST PUSH1 0 JUMP`: loops until the transaction runs out of gas.
const LOOP_CONTRACT: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xc0, 0xde,
]);
const LOOP_CODE: [u8; 4] = [0x5b, 0x60, 0x00, 0x56];

/// Each transaction gets a tenth of the gas limit, so the eleventh to finish is over it.
const GAS_LIMIT_FRACTION: u64 = 10;
/// Twenty gas limits' worth of transactions.
const TX_COUNT: usize = 200;

const MAX_FEE_PER_GAS: u64 = 10_000_000_000;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// One sender per transaction, each at nonce 0, so every transaction is valid against
/// the pre-state on its own and the only thing wrong with the block is its total gas.
fn sender_key(index: usize) -> SecretKey {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&(index as u64 + 1).to_be_bytes());
    SecretKey::from_slice(&bytes).expect("small scalars are valid secret keys")
}

async fn setup_store(senders: &[Address]) -> Store {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-bal.json"))
        .expect("open l1-bal genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-bal genesis");
    genesis.alloc.insert(
        LOOP_CONTRACT,
        GenesisAccount {
            code: Bytes::from_static(&LOOP_CODE),
            storage: Default::default(),
            balance: U256::zero(),
            nonce: 0,
        },
    );
    for sender in senders {
        genesis.alloc.insert(
            *sender,
            GenesisAccount {
                code: Bytes::new(),
                storage: Default::default(),
                balance: U256::from(10u64).pow(U256::from(18u64)),
                nonce: 0,
            },
        );
    }
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    store
}

/// A valid empty Amsterdam block on top of genesis, and its BAL. Its header supplies a
/// gas limit, base fee and slot number that pass header validation.
async fn build_empty_block(store: &Store) -> (Block, BlockAccessList) {
    let blockchain = Blockchain::new(store.clone(), BlockchainOptions::default());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(1),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, store, Bytes::new()).unwrap();
    let result = blockchain.build_payload(payload).unwrap();
    let bal = result
        .block_access_list
        .expect("amsterdam block must produce a BAL");
    (result.payload, bal)
}

#[tokio::test]
async fn parallel_path_stops_once_completed_transactions_exceed_the_gas_limit() {
    let signers: Vec<Signer> = (0..TX_COUNT)
        .map(|index| LocalSigner::new(sender_key(index)).into())
        .collect();
    let senders: Vec<Address> = signers.iter().map(Signer::address).collect();

    let build_store = setup_store(&senders).await;
    let chain_id = build_store.get_chain_config().chain_id;
    let (mut block, bal) = build_empty_block(&build_store).await;
    let gas_limit = block.header.gas_limit;

    let mut transactions = Vec::with_capacity(TX_COUNT);
    for signer in &signers {
        let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            chain_id,
            nonce: 0,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: MAX_FEE_PER_GAS,
            gas_limit: gas_limit / GAS_LIMIT_FRACTION,
            to: TxKind::Call(LOOP_CONTRACT),
            value: U256::zero(),
            ..Default::default()
        });
        tx.sign_inplace(signer).await.unwrap();
        transactions.push(tx);
    }

    // The pre-execution check must let this block through, so that the rejection below
    // can only come from stopping during execution.
    check_minimum_block_work(
        transactions.iter().zip(senders.iter().copied()),
        Fork::Amsterdam,
        gas_limit,
    )
    .expect("the block is small enough to pass the minimum-work check");

    block.header.transactions_root = compute_transactions_root(&transactions, &NativeCrypto);
    block.header.hash = Default::default();
    block.body.transactions = transactions;

    let blockchain = Blockchain::new(
        setup_store(&senders).await,
        BlockchainOptions {
            bal_parallel_exec_enabled: true,
            ..Default::default()
        },
    );
    let result = blockchain.add_block_pipeline_bal(block, Some(Arc::new(bal)));

    let err = format!(
        "{:?}",
        result.expect_err("an over-limit block must be rejected")
    );
    assert!(
        err.contains("transactions completed during parallel execution"),
        "must be rejected by the early stop, not by ordered admission after executing \
         every transaction, got: {err}"
    );
}
