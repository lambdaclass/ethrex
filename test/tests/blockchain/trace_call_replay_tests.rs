//! `debug_traceCall` on the re-execution path, where the state has to be rebuilt by
//! replaying the block's own transactions.
//!
//! These live at the blockchain layer rather than the RPC layer because the RPC layer
//! bounds `txIndex < tx_count`, and the differential test below needs the one pairing
//! that bound forbids: `tx_index == tx_count` against `tx_index == None`, two routes to
//! the same state.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Duration;

use bytes::Bytes;
use ethrex_blockchain::tracing::TraceCallOverrides;
use ethrex_blockchain::vm::StateOverride;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    tracing::CallTrace,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER,
        GenericTransaction, Genesis, GenesisAccount, Transaction, TxKind,
    },
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

const TEST_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
const RECIPIENT: Address = H160([0xbb; 20]);
/// Address of the probe contract added to genesis by [`chain_with_two_txs`].
const BALANCE_READER: Address = H160([0xcc; 20]);
const TIMEOUT: Duration = Duration::from_secs(30);
const REEXEC: u32 = 128;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn signer() -> (Signer, Address) {
    let sk = SecretKey::from_slice(&hex::decode(TEST_PRIVATE_KEY).unwrap()).unwrap();
    // `address` is a public field on LocalSigner, not a method; Signer is built via Into.
    let address = LocalSigner::new(sk).address;
    (LocalSigner::new(sk).into(), address)
}

/// `PUSH20 <RECIPIENT>; BALANCE; PUSH1 0; MSTORE; PUSH1 32; PUSH1 0; RETURN`
///
/// Returns `RECIPIENT`'s balance as the call's 32-byte output, which turns a trace into a
/// readout of the state the traced call actually saw. Straight-line code, no jumps.
fn balance_reader_code() -> Bytes {
    let mut code = vec![0x73]; // PUSH20
    code.extend_from_slice(RECIPIENT.as_bytes());
    code.extend_from_slice(&[
        0x31, // BALANCE
        0x60, 0x00, // PUSH1 0
        0x52, // MSTORE
        0x60, 0x20, // PUSH1 32
        0x60, 0x00, // PUSH1 0
        0xf3, // RETURN
    ]);
    Bytes::from(code)
}

/// Genesis plus one executed canonical block carrying two signed transfers.
///
/// The block's withdrawal list is **empty**, which the differential test depends on:
/// `rerun_block` applies withdrawals only when `stop_index.is_none()`, so a block with
/// actual withdrawals would legitimately trace differently at `tx_index == tx_count`.
async fn chain_with_two_txs() -> (Store, Blockchain, Block, Address) {
    let (signer, sender) = signer();

    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("genesis fixture");
    let mut genesis: Genesis = serde_json::from_reader(BufReader::new(file)).expect("genesis json");
    // GenesisAccount derives no Default — every field is spelled out.
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
        BALANCE_READER,
        GenesisAccount {
            code: balance_reader_code(),
            storage: Default::default(),
            balance: U256::zero(),
            nonce: 0,
        },
    );
    let chain_id = genesis.config.chain_id;

    // `add_initial_state` takes `&mut self`.
    let mut store = Store::new("trace-replay-test", EngineType::InMemory).expect("in-memory store");
    store.add_initial_state(genesis).await.unwrap();
    let blockchain = Blockchain::default_with_store(store.clone());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    for nonce in 0..2u64 {
        let mut tx = EIP1559Transaction {
            chain_id,
            nonce,
            max_priority_fee_per_gas: 1_000_000_000,
            max_fee_per_gas: 10_000_000_000,
            gas_limit: 100_000,
            to: TxKind::Call(RECIPIENT),
            value: U256::from(1_000u64),
            ..Default::default()
        };
        tx.sign_inplace(&signer).await.unwrap();
        blockchain
            .add_transaction_to_pool(Transaction::EIP1559Transaction(tx))
            .await
            .expect("tx enters pool");
    }

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
        2,
        "fixture must produce a block with two transactions"
    );

    let (number, hash) = (block.header.number, block.hash());
    blockchain.add_block(block.clone()).expect("block is valid");
    store
        .forkchoice_update(vec![], number, hash, None, None)
        .await
        .unwrap();

    // Fixture premise: both transfers actually landed. Every assertion below rests on
    // this, so it must not be an unchecked assumption.
    let recipient = store
        .get_account_state_by_root(block.header.state_root, RECIPIENT)
        .unwrap()
        .expect("recipient exists on chain");
    assert_eq!(
        recipient.balance,
        U256::from(2_000u64),
        "fixture premise: both transfers landed"
    );

    (store, blockchain, block, sender)
}

fn call_reader(sender: Address) -> GenericTransaction {
    GenericTransaction {
        from: sender,
        to: TxKind::Call(BALANCE_READER),
        gas: Some(100_000),
        ..Default::default()
    }
}

/// The balance the traced call observed for `RECIPIENT`, read out of the trace's output.
fn observed_recipient_balance(trace: &CallTrace) -> U256 {
    U256::from_big_endian(&trace[0].output)
}

fn overrides_setting_balance(address: Address, balance: u64) -> TraceCallOverrides {
    let mut state = BTreeMap::new();
    state.insert(
        address,
        StateOverride {
            balance: Some(U256::from(balance)),
            ..Default::default()
        },
    );
    TraceCallOverrides {
        state,
        real_head_number: 1,
        effective_header: None,
    }
}

/// The §4 invariant, end to end.
///
/// `tx_index = Some(len)` reaches the block's post-state by replaying it through
/// `ReplayedVmDatabase`; `tx_index = None` reads the stored post-state directly. Same
/// state, two routes, so the traces must be identical — with overrides and without. The
/// traced call reads `BALANCE(RECIPIENT)`, so its output is state-derived: a
/// `ReplayedVmDatabase` bug that got a balance wrong would show up in the compared JSON.
#[tokio::test]
async fn replayed_state_traces_identically_to_stored_state() {
    let (_store, blockchain, block, sender) = chain_with_two_txs().await;
    let tx_count = block.body.transactions.len();

    for overrides in [
        TraceCallOverrides::default(),
        overrides_setting_balance(RECIPIENT, 12_345),
    ] {
        let via_stored = blockchain
            .trace_call_calls(
                block.clone(),
                None,
                call_reader(sender),
                REEXEC,
                TIMEOUT,
                false,
                false,
                overrides.clone(),
            )
            .await
            .expect("stored-state path");

        let via_replay = blockchain
            .trace_call_calls(
                block.clone(),
                Some(tx_count),
                call_reader(sender),
                REEXEC,
                TIMEOUT,
                false,
                false,
                overrides,
            )
            .await
            .expect("replay path");

        assert_eq!(
            serde_json::to_value(&via_replay).unwrap(),
            serde_json::to_value(&via_stored).unwrap(),
            "replayed state must trace identically to the stored post-state"
        );
    }
}

/// The override must reach the traced call on the replay path.
///
/// The probe contract returns `BALANCE(RECIPIENT)`, so the trace's output *is* the balance
/// the call saw. Without an override it must be the replayed 2000; with one it must be the
/// overridden value. If overrides were silently dropped on this path, the second assertion
/// fails.
#[tokio::test]
async fn override_reaches_the_traced_call_on_the_replay_path() {
    let (_store, blockchain, block, sender) = chain_with_two_txs().await;
    let tx_count = block.body.transactions.len();

    let baseline = blockchain
        .trace_call_calls(
            block.clone(),
            Some(tx_count),
            call_reader(sender),
            REEXEC,
            TIMEOUT,
            false,
            false,
            TraceCallOverrides::default(),
        )
        .await
        .expect("replay path without overrides");
    assert_eq!(
        observed_recipient_balance(&baseline),
        U256::from(2_000u64),
        "the replayed state should show both transfers"
    );

    let overridden = blockchain
        .trace_call_calls(
            block.clone(),
            Some(tx_count),
            call_reader(sender),
            REEXEC,
            TIMEOUT,
            false,
            false,
            overrides_setting_balance(RECIPIENT, 999),
        )
        .await
        .expect("replay path with overrides");
    assert_eq!(
        observed_recipient_balance(&overridden),
        U256::from(999u64),
        "the traced call must see the override, not the replayed balance"
    );
}

/// The override must NOT be visible to the block's own transactions as they replay.
///
/// This is the regression the two-`Evm` split exists to prevent. The sender's balance is
/// overridden to 1 wei — far too little to have paid for the block's two transfers. The
/// override applies to the traced call only, so the replay must be unaffected and
/// `RECIPIENT` must still reach 2000. If the overlay leaked into the replay, transaction 0
/// would fail for insufficient funds and the probe would read 0 (or the trace would error).
#[tokio::test]
async fn override_is_invisible_to_the_replayed_transactions() {
    let (_store, blockchain, block, sender) = chain_with_two_txs().await;
    let tx_count = block.body.transactions.len();

    let trace = blockchain
        .trace_call_calls(
            block.clone(),
            Some(tx_count),
            call_reader(sender),
            REEXEC,
            TIMEOUT,
            false,
            false,
            overrides_setting_balance(sender, 1),
        )
        .await
        .expect("an override on the sender must not break the replay");

    assert_eq!(
        observed_recipient_balance(&trace),
        U256::from(2_000u64),
        "the block's own transactions must have executed against the real state"
    );
}
