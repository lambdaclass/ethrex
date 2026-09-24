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
    evm::calculate_create_address,
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
/// Address of the probe contract added to genesis by [`chain_with_three_txs`].
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

/// `PUSH1 0; SLOAD; PUSH1 0; MSTORE; PUSH1 32; PUSH1 0; RETURN`
///
/// Runtime code for the contract [`storage_probe_init_code`] deploys. Reads `storage[0]`
/// and returns it as the call's 32-byte output, so a traced call against the deployed
/// address reads back whatever the constructor wrote — this is what exercises
/// `added_storage` on the replay path, rather than only `get_account_state`'s
/// balance/nonce fields.
fn storage_probe_runtime_code() -> Vec<u8> {
    vec![
        0x60, 0x00, // PUSH1 0
        0x54, // SLOAD
        0x60, 0x00, // PUSH1 0
        0x52, // MSTORE
        0x60, 0x20, // PUSH1 32
        0x60, 0x00, // PUSH1 0
        0xf3, // RETURN
    ]
}

/// `PUSH1 42; PUSH1 0; SSTORE; PUSH11 <runtime>; PUSH1 0; MSTORE; PUSH1 11; PUSH1 21; RETURN`
///
/// Init code: writes `storage[0] = 42` in the constructor, then returns
/// [`storage_probe_runtime_code`] as the deployed code. `PUSH11 <runtime>` leaves the
/// 11-byte runtime code right-aligned in a 32-byte stack word, so `MSTORE`-ing it at
/// memory offset 0 lands the code in bytes 21..32; `RETURN(21, 11)` returns exactly that
/// slice.
///
/// The deployed contract's runtime code has to be readable for the call to execute at
/// all, and its constructor's storage write has to be readable for the output to be 42
/// rather than 0 — but only the storage half is actually differential coverage. See the
/// note on `chain_with_three_txs`'s deploy transaction for why `get_account_code` /
/// `get_code_metadata` can't be pinned down this way, no matter how this contract is
/// shaped.
fn storage_probe_init_code() -> Bytes {
    let runtime = storage_probe_runtime_code();
    assert_eq!(runtime.len(), 11, "PUSH11 below assumes an 11-byte runtime");

    let mut code = vec![
        0x60, 0x2a, // PUSH1 42
        0x60, 0x00, // PUSH1 0
        0x55, // SSTORE
        0x6a, // PUSH11
    ];
    code.extend_from_slice(&runtime);
    code.extend_from_slice(&[
        0x60, 0x00, // PUSH1 0
        0x52, // MSTORE
        0x60, 0x0b, // PUSH1 11
        0x60, 0x15, // PUSH1 21
        0xf3, // RETURN
    ]);
    Bytes::from(code)
}

/// Genesis plus one executed canonical block carrying two signed transfers and a
/// contract-creation transaction.
///
/// The creation transaction deploys [`storage_probe_init_code`]'s contract, which writes
/// `storage[0] = 42` in its constructor and returns that slot's value on any later call —
/// giving the differential test in-block code and storage to exercise, not just the
/// balance/nonce fields the two transfers touch.
///
/// The block's withdrawal list is **empty**, which the differential test depends on:
/// `rerun_block` applies withdrawals only when `stop_index.is_none()`, so a block with
/// actual withdrawals would legitimately trace differently at `tx_index == tx_count`.
///
/// Returns `(store, blockchain, block, sender, deployed_contract_address)`.
async fn chain_with_three_txs() -> (Store, Blockchain, Block, Address, Address) {
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

    // This transaction gives the differential test below real `added_storage` coverage
    // (confirmed by mutation — see `replayed_state_traces_identically_to_stored_state`),
    // but it cannot give it `get_account_code`/`get_code_metadata` coverage, no matter
    // how the deployed contract is shaped. `Store::get_account_code` is keyed by content
    // hash alone, independent of any state root (`crates/storage/store.rs:1173`). The
    // `blockchain.add_block` call below, which produces the stored ground truth for the
    // `tx_index: None` side of the comparison, also writes this contract's code into
    // that global, hash-addressed table — so on the replay side, `ReplayedVmDatabase`'s
    // inner database (anchored at the block's *parent*) resolves the code hash
    // successfully whether or not its own scan over `updates` ever runs. Verified by
    // disabling that scan outright and rerunning every test in this file: all four still
    // passed. Two routes to the same *state* are not two routes to the same *code
    // table* — code is global, not per-state — so this differential shape cannot isolate
    // the scan. Its coverage is the direct unit test `code_deployed_by_the_replay_is_served`
    // (`crates/blockchain/vm.rs`), which wraps an inner database that genuinely lacks the
    // code. Don't try to "close" this gap with a fancier fixture here; it will pass
    // either way and prove nothing.
    let deploy_nonce = 2u64;
    let deployed_address = calculate_create_address(sender, deploy_nonce);
    let mut deploy_tx = EIP1559Transaction {
        chain_id,
        nonce: deploy_nonce,
        max_priority_fee_per_gas: 1_000_000_000,
        max_fee_per_gas: 10_000_000_000,
        gas_limit: 300_000,
        to: TxKind::Create,
        value: U256::zero(),
        data: storage_probe_init_code(),
        ..Default::default()
    };
    deploy_tx.sign_inplace(&signer).await.unwrap();
    blockchain
        .add_transaction_to_pool(Transaction::EIP1559Transaction(deploy_tx))
        .await
        .expect("deploy tx enters pool");

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
        3,
        "fixture must produce a block with three transactions"
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

    // Fixture premise: the deployed contract exists and has non-empty code. This is the
    // `calculate_create_address` derivation checking itself against the actual chain
    // state, not just the formula.
    let deployed = store
        .get_account_state_by_root(block.header.state_root, deployed_address)
        .unwrap()
        .expect("deployed contract exists on chain");
    assert_ne!(
        deployed.code_hash,
        *ethrex_common::constants::EMPTY_KECCAK_HASH,
        "fixture premise: the deploy transaction must have left code behind"
    );

    (store, blockchain, block, sender, deployed_address)
}

fn call_reader(sender: Address) -> GenericTransaction {
    GenericTransaction {
        from: sender,
        to: TxKind::Call(BALANCE_READER),
        gas: Some(100_000),
        ..Default::default()
    }
}

fn call_storage_probe(sender: Address, deployed_address: Address) -> GenericTransaction {
    GenericTransaction {
        from: sender,
        to: TxKind::Call(deployed_address),
        gas: Some(100_000),
        ..Default::default()
    }
}

/// The balance the traced call observed for `RECIPIENT`, read out of the trace's output.
fn observed_recipient_balance(trace: &CallTrace) -> U256 {
    U256::from_big_endian(&trace[0].output)
}

/// `storage[0]` as the storage-probe contract observed it, read out of the trace's output.
fn observed_storage_probe_output(trace: &CallTrace) -> U256 {
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
        state: std::sync::Arc::new(state),
        effective_header: None,
    }
}

/// Isolates the fixture's own correctness from `ReplayedVmDatabase`.
///
/// This traces the deployed contract with `tx_index = None`, i.e. entirely off the
/// replay path: the call runs against the *stored* post-state, straight out of the
/// trie. If this fails, the bug is in `storage_probe_init_code`/`storage_probe_runtime_code`
/// or in the `calculate_create_address` derivation — not in this branch's
/// `ReplayedVmDatabase`. [`replayed_state_traces_identically_to_stored_state`] is the
/// test that puts `ReplayedVmDatabase` itself on trial, and it only means something once
/// this one passes.
#[tokio::test]
async fn deployed_contract_storage_reads_correctly_from_stored_state() {
    let (_store, blockchain, block, sender, deployed_address) = chain_with_three_txs().await;

    let trace = blockchain
        .trace_call_calls(
            block,
            None,
            call_storage_probe(sender, deployed_address),
            REEXEC,
            TIMEOUT,
            false,
            false,
            TraceCallOverrides::default(),
        )
        .await
        .expect("stored-state path");

    assert_eq!(
        observed_storage_probe_output(&trace),
        U256::from(42u64),
        "the constructor's storage write must be readable back from the stored post-state"
    );
}

/// The §4 invariant, end to end.
///
/// `tx_index = Some(len)` reaches the block's post-state by replaying it through
/// `ReplayedVmDatabase`; `tx_index = None` reads the stored post-state directly. Same
/// state, two routes, so the traces must be identical — with overrides and without.
///
/// Two probes run through both routes: `call_reader` reads `BALANCE(RECIPIENT)`, so its
/// output is state-derived from `get_account_state`'s balance path; `call_storage_probe`
/// targets the contract [`chain_with_three_txs`] deploys and reads back its
/// constructor's storage write, exercising `get_storage_slot`'s `added_storage` path
/// (see [`deployed_contract_storage_reads_correctly_from_stored_state`] for the
/// fixture's own correctness, checked independently of `ReplayedVmDatabase`). A
/// `ReplayedVmDatabase` bug in either path would show up in the compared JSON for one
/// probe or the other.
///
/// This does **not** cover `get_account_code`/`get_code_metadata`'s scan over
/// `updates`, and cannot: see the comment on the deploy transaction in
/// `chain_with_three_txs` for why code lookups can't be pinned to the replay route by a
/// two-routes-to-one-state comparison. That coverage is
/// `code_deployed_by_the_replay_is_served` in `crates/blockchain/vm.rs` instead. Nor
/// does it cover `removed`/`removed_storage` — this fixture has no `SELFDESTRUCT` and no
/// destroyed-and-recreated account; that coverage is likewise the `vm.rs` unit tests
/// (`removed_account_is_absent_and_reads_zero_storage`,
/// `removed_storage_closes_the_world`,
/// `destroyed_account_with_nothing_written_back_reports_no_storage`).
#[tokio::test]
async fn replayed_state_traces_identically_to_stored_state() {
    let (_store, blockchain, block, sender, deployed_address) = chain_with_three_txs().await;
    let tx_count = block.body.transactions.len();

    for overrides in [
        TraceCallOverrides::default(),
        overrides_setting_balance(RECIPIENT, 12_345),
    ] {
        for call in [
            call_reader(sender),
            call_storage_probe(sender, deployed_address),
        ] {
            let via_stored = blockchain
                .trace_call_calls(
                    block.clone(),
                    None,
                    call.clone(),
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
                    call,
                    REEXEC,
                    TIMEOUT,
                    false,
                    false,
                    overrides.clone(),
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
}

/// The override must reach the traced call on the replay path.
///
/// The probe contract returns `BALANCE(RECIPIENT)`, so the trace's output *is* the balance
/// the call saw. Without an override it must be the replayed 2000; with one it must be the
/// overridden value. If overrides were silently dropped on this path, the second assertion
/// fails.
#[tokio::test]
async fn override_reaches_the_traced_call_on_the_replay_path() {
    let (_store, blockchain, block, sender, _deployed_address) = chain_with_three_txs().await;
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
    let (_store, blockchain, block, sender, _deployed_address) = chain_with_three_txs().await;
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
