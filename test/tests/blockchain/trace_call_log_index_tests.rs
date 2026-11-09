//! `debug_traceCall` `withLog` numbering: no receipt exists for the traced call, so its
//! first log gets index 0 when it runs on top of the block. Only the transactions the
//! replay path re-executes before it (`tx_index = Some(i)`) seed the index, which is
//! what geth's `StateAtTransaction` statedb does.

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Duration;

use bytes::Bytes;
use ethrex_blockchain::tracing::TraceCallOverrides;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER, GenericTransaction,
        Genesis, GenesisAccount, Transaction, TxKind,
    },
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

const TEST_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
/// Genesis contract running `PUSH1 0; PUSH1 0; LOG0; STOP`.
const LOGGER: Address = H160([0xdd; 20]);
const TIMEOUT: Duration = Duration::from_secs(30);
const REEXEC: u32 = 128;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn call_logger(sender: Address) -> GenericTransaction {
    GenericTransaction {
        from: sender,
        to: TxKind::Call(LOGGER),
        gas: Some(100_000),
        ..Default::default()
    }
}

/// `debug_traceCall` on top of a block whose only transaction emitted one log: the
/// traced call's log is index 0. Tracing after replaying that transaction
/// (`tx_index = Some(1)`) counts its log, so the same call gets index 1.
#[tokio::test]
async fn trace_call_log_index_starts_at_zero_on_top_of_the_block() {
    let sk = SecretKey::from_slice(&hex::decode(TEST_PRIVATE_KEY).unwrap()).unwrap();
    let sender = LocalSigner::new(sk).address;
    let signer: Signer = LocalSigner::new(sk).into();

    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("genesis fixture");
    let mut genesis: Genesis = serde_json::from_reader(BufReader::new(file)).expect("genesis json");
    genesis.alloc.insert(
        sender,
        GenesisAccount {
            code: Bytes::new(),
            storage: Default::default(),
            balance: U256::from(10u64).pow(U256::from(20)),
            nonce: 0,
        },
    );
    genesis.alloc.insert(
        LOGGER,
        GenesisAccount {
            code: Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xa0, 0x00]),
            storage: Default::default(),
            balance: U256::zero(),
            nonce: 0,
        },
    );
    let chain_id = genesis.config.chain_id;

    let mut store =
        Store::new("trace-call-log-index", EngineType::InMemory).expect("in-memory store");
    store.add_initial_state(genesis).await.unwrap();
    let blockchain = Blockchain::default_with_store(store.clone());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    let mut tx = EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 1_000_000_000,
        max_fee_per_gas: 10_000_000_000,
        gas_limit: 100_000,
        to: TxKind::Call(LOGGER),
        ..Default::default()
    };
    tx.sign_inplace(&signer).await.unwrap();
    blockchain
        .add_transaction_to_pool(Transaction::EIP1559Transaction(tx))
        .await
        .expect("tx enters pool");

    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let block = blockchain.build_payload(payload).unwrap().payload;
    assert_eq!(
        block.body.transactions.len(),
        1,
        "fixture block carries the logging tx"
    );
    let (number, hash) = (block.header.number, block.hash());
    blockchain.add_block(block.clone()).expect("block is valid");
    store
        .forkchoice_update(vec![], number, hash, None, None)
        .await
        .unwrap();

    let on_top = blockchain
        .trace_call_calls(
            block.clone(),
            None,
            call_logger(sender),
            REEXEC,
            TIMEOUT,
            false,
            /* with_log */ true,
            TraceCallOverrides::default(),
        )
        .await
        .expect("trace on top of the block");
    assert_eq!(on_top[0].logs.len(), 1, "the traced call emits one log");
    assert_eq!(
        on_top[0].logs[0].index, 0,
        "no receipt precedes a traced call"
    );

    let after_replay = blockchain
        .trace_call_calls(
            block,
            Some(1),
            call_logger(sender),
            REEXEC,
            TIMEOUT,
            false,
            /* with_log */ true,
            TraceCallOverrides::default(),
        )
        .await
        .expect("trace after replaying tx 0");
    assert_eq!(
        after_replay[0].logs[0].index, 1,
        "the replayed tx's log counts, as on geth's replayed statedb"
    );
}
