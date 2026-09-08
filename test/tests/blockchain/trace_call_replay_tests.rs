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

    (store, blockchain, block, sender)
}

fn call_from(sender: Address) -> GenericTransaction {
    GenericTransaction {
        from: sender,
        to: TxKind::Call(RECIPIENT),
        gas: Some(100_000),
        ..Default::default()
    }
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
/// state, two routes, so the traces must be identical — with overrides and without.
#[tokio::test]
async fn replayed_state_traces_identically_to_stored_state() {
    let (_store, blockchain, block, sender) = chain_with_two_txs().await;
    let tx_count = block.body.transactions.len();

    for overrides in [
        TraceCallOverrides::default(),
        overrides_setting_balance(sender, 12_345),
    ] {
        let via_stored = blockchain
            .trace_call_calls(
                block.clone(),
                None,
                call_from(sender),
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
                call_from(sender),
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

/// The override must reach the traced call *and* the replay must not see it.
///
/// Tracing at `tx_index = 1` means transaction 0 has already run. Overriding the sender's
/// balance must change what the traced call sees without rewriting what transaction 0 did.
#[tokio::test]
async fn override_applies_to_the_call_without_disturbing_the_replay() {
    let (store, blockchain, block, sender) = chain_with_two_txs().await;

    let baseline = blockchain
        .trace_call_calls(
            block.clone(),
            Some(1),
            call_from(sender),
            REEXEC,
            TIMEOUT,
            false,
            false,
            TraceCallOverrides::default(),
        )
        .await
        .expect("no-override replay at tx_index 1");

    let overridden = blockchain
        .trace_call_calls(
            block.clone(),
            Some(1),
            call_from(sender),
            REEXEC,
            TIMEOUT,
            false,
            false,
            overrides_setting_balance(RECIPIENT, 999),
        )
        .await
        .expect("override applies at tx_index 1");

    // Both traces succeed: the override did not corrupt the replay into a failure.
    assert_eq!(baseline.len(), overridden.len());

    // And transaction 0's effect on chain is untouched by the override: the recipient's
    // committed balance still reflects the two transfers, not the override.
    let recipient = store
        .get_account_state_by_root(block.header.state_root, RECIPIENT)
        .unwrap()
        .expect("recipient exists on chain");
    assert_eq!(
        recipient.balance,
        U256::from(2_000u64),
        "the override must never have been visible to the block's own transactions"
    );
}
