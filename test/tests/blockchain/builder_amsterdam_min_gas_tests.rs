//! The payload builder's early exit when little gas is left.
//!
//! The builder ended a block as soon as fewer than 21,000 gas remained, the pre-Amsterdam
//! minimum. EIP-2780 lowers a self-transfer's intrinsic gas to 12,000 at Amsterdam, so a block
//! ending with 12,000 to 20,999 gas left omitted a pending self-transfer that fits. The
//! threshold follows the fork's lowest intrinsic gas.

use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        EIP1559Transaction, ELASTICITY_MULTIPLIER, Genesis, GenesisAccount, Transaction, TxKind,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

/// A contract whose only instruction is INVALID: a call to it burns the call's whole gas limit.
const BURNER: Address = H160([0xfe; 20]);
const BLOCK_GAS_LIMIT: u64 = 100_000;
/// Gas the filler leaves: below the old 21,000 threshold, above what a self-transfer needs.
const GAS_LEFT_AFTER_FILLER: u64 = 16_000;
/// EIP-2780: an ETH transfer to the sender's own address costs 12,000 gas at Amsterdam.
const SELF_TRANSFER_GAS: u64 = 12_000;
const MAX_FEE_PER_GAS: u64 = 10_000_000_000;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// One signer per key byte; `k` must be nonzero.
fn signer(k: u8) -> Signer {
    LocalSigner::new(SecretKey::from_slice(&[k; 32]).unwrap()).into()
}

async fn setup_store(funded: &[Address]) -> Store {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-bal.json"))
        .expect("open l1-bal genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-bal genesis");
    genesis.gas_limit = BLOCK_GAS_LIMIT;
    for funded in funded.iter().copied() {
        genesis.alloc.insert(
            funded,
            GenesisAccount {
                code: Bytes::new(),
                storage: Default::default(),
                balance: U256::from(10u64).pow(U256::from(18u64)),
                nonce: 0,
            },
        );
    }
    genesis.alloc.insert(
        BURNER,
        GenesisAccount {
            code: Bytes::from(vec![0xfe]),
            storage: Default::default(),
            balance: U256::zero(),
            nonce: 0,
        },
    );
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    store
}

/// `signer`'s first transaction, at nonce 0.
async fn first_transaction(
    store: &Store,
    signer: &Signer,
    to: Address,
    gas_limit: u64,
    tip: u64,
) -> Transaction {
    let chain_id = store.get_chain_config().chain_id;
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: tip,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        gas_limit,
        to: TxKind::Call(to),
        value: U256::zero(),
        ..Default::default()
    });
    tx.sign_inplace(signer).await.unwrap();
    tx
}

/// A filler leaves 16,000 gas; the pending 12,000-gas self-transfer fits and is included.
#[tokio::test]
async fn builder_includes_a_self_transfer_in_the_last_gas_below_21000_at_amsterdam() {
    let filler_sender = signer(1);
    let self_sender = signer(2);
    let store = setup_store(&[filler_sender.address(), self_sender.address()]).await;
    let blockchain = Blockchain::new(store.clone(), BlockchainOptions::default());
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let args = BuildPayloadArgs {
        parent: genesis.hash(),
        timestamp: genesis.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(1),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: BLOCK_GAS_LIMIT,
    };
    let payload = create_payload(&args, &store, Bytes::new()).expect("create payload");
    let gas_limit = payload.header.gas_limit;

    // The filler burns everything but GAS_LEFT_AFTER_FILLER and pays the higher tip, so the
    // builder takes it first.
    let filler = first_transaction(
        &store,
        &filler_sender,
        BURNER,
        gas_limit - GAS_LEFT_AFTER_FILLER,
        10,
    )
    .await;
    let self_transfer = first_transaction(
        &store,
        &self_sender,
        self_sender.address(),
        SELF_TRANSFER_GAS,
        1,
    )
    .await;
    blockchain
        .add_transaction_to_pool(filler)
        .await
        .expect("the filler is admitted");
    blockchain
        .add_transaction_to_pool(self_transfer.clone())
        .await
        .expect("a 12,000-gas self-transfer is admitted at Amsterdam");

    let built = blockchain.build_payload(payload).expect("build payload");
    let txs = &built.payload.body.transactions;
    assert_eq!(
        txs.len(),
        2,
        "the self-transfer fits in the {GAS_LEFT_AFTER_FILLER} gas the filler leaves"
    );
    assert_eq!(
        txs[1].hash(&NativeCrypto),
        self_transfer.hash(&NativeCrypto),
        "the second transaction is the self-transfer"
    );
    assert_eq!(
        built.payload.header.gas_used,
        gas_limit - GAS_LEFT_AFTER_FILLER + SELF_TRANSFER_GAS
    );
}
