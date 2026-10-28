//! The parallel Amsterdam path keeps one deferred BAL validation error for the whole
//! block, the earliest failing transaction's, and stops validating transactions behind it.
//!
//! It used to hold one error per transaction until every transaction had run. A block
//! access list with one bogus account that no transaction touches, declared changed at
//! every transaction, failed every transaction's validation, and each failure's message
//! was retained, so one invalid payload cost memory proportional to the transaction count
//! times the account's entry before it was rejected.

use std::{fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    error::ChainError,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    constants::BAL_ITEM_COST,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER, Genesis,
        GenesisAccount, Transaction, TxKind,
        block_access_list::{
            AccountChanges, BalanceChange, BlockAccessList, SlotChange, StorageChange,
        },
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::{EarliestBalError, EvmError};
use secp256k1::SecretKey;

const MAX_FEE_PER_GAS: u64 = 10_000_000_000;

/// Codeless recipient of every transaction.
const SINK: Address = H160([0x11; 20]);
/// The bogus account: no transaction touches it.
const BOGUS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x01, 0x00,
]);

const TX_COUNT: usize = 40;
const BOGUS_SLOTS: u64 = 200;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

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
    let mut store = Store::new("store.db", EngineType::InMemory).expect("in-memory store");
    store.add_initial_state(genesis).await.expect("genesis");
    store
}

/// A valid Amsterdam block of `TX_COUNT` plain transfers, built by the block builder,
/// with the BAL it produced and the senders' addresses.
async fn honest_block() -> (Block, BlockAccessList, Vec<Address>) {
    let signers: Vec<Signer> = (0..TX_COUNT)
        .map(|i| LocalSigner::new(sender_key(i)).into())
        .collect();
    let senders: Vec<Address> = signers.iter().map(Signer::address).collect();
    let store = setup_store(&senders).await;
    let chain_id = store.get_chain_config().chain_id;
    let mut txs = Vec::with_capacity(TX_COUNT);
    for signer in &signers {
        let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            chain_id,
            nonce: 0,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: MAX_FEE_PER_GAS,
            gas_limit: 100_000,
            to: TxKind::Call(SINK),
            value: U256::zero(),
            ..Default::default()
        });
        tx.sign_inplace(signer).await.unwrap();
        txs.push(tx);
    }
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
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let result = blockchain
        .build_payload_with_transactions(payload, txs)
        .expect("honest block builds");
    assert_eq!(result.payload.body.transactions.len(), TX_COUNT);
    let bal = result.block_access_list.expect("amsterdam BAL");
    (result.payload, bal, senders)
}

/// The honest BAL plus the bogus account: a distinct balance change at every transaction
/// index (so the shorter "no-op" rejection never fires first) and `BOGUS_SLOTS` slots
/// each changed at every index. Canonically ordered and far under the EIP-7928 item cap,
/// so nothing before execution rejects it.
fn forge(honest: &BlockAccessList) -> BlockAccessList {
    let mut accounts: Vec<AccountChanges> = honest.accounts().to_vec();
    let balance_changes: Vec<BalanceChange> = (1..=TX_COUNT as u32)
        .map(|i| BalanceChange::new(i, U256::from(i + 1)))
        .collect();
    let storage_changes: Vec<SlotChange> = (0..BOGUS_SLOTS)
        .map(|k| {
            let changes = (1..=TX_COUNT as u32 + 1)
                .map(|j| StorageChange::new(j, U256::one()))
                .collect();
            SlotChange::with_changes(U256::from(k), changes)
        })
        .collect();
    accounts.push(
        AccountChanges::new(BOGUS)
            .with_storage_changes(storage_changes)
            .with_balance_changes(balance_changes),
    );
    accounts.sort_by_key(|a| a.address);
    BlockAccessList::from_accounts(accounts)
}

/// Importing a block whose BAL names an account no transaction touches fails on the first
/// transaction, with a message that does not grow with the bogus account's entry.
#[tokio::test]
async fn bogus_bal_account_is_rejected_with_one_bounded_error() {
    let (honest_block, honest_bal, senders) = honest_block().await;
    let forged = forge(&honest_bal);
    forged
        .validate_ordering()
        .expect("the forged BAL is canonically ordered");
    assert!(forged.item_count() <= honest_block.header.gas_limit / BAL_ITEM_COST);

    let mut block = honest_block;
    block.header.block_access_list_hash = Some(forged.compute_hash(&NativeCrypto));
    block.header.hash = Default::default();

    let blockchain = Blockchain::new(
        setup_store(&senders).await,
        BlockchainOptions {
            bal_parallel_exec_enabled: true,
            ..Default::default()
        },
    );
    let err = blockchain
        .add_block_pipeline(block, Some(Arc::new(forged)))
        .expect_err("the forged BAL must be rejected");
    let ChainError::EvmError(EvmError::Custom(msg)) = err else {
        panic!("expected a BAL validation error, got: {err:?}");
    };
    assert!(
        msg.starts_with("BAL validation failed for tx 0:"),
        "the earliest failing transaction's error is reported, got: {msg}"
    );
    assert!(
        msg.contains("has BAL balance change at 1 but not in execution state"),
        "got: {msg}"
    );
    assert!(
        msg.len() < 1024,
        "the error must not grow with the bogus entry, got {} bytes",
        msg.len()
    );
}

/// The shared slot keeps the lowest-index error whatever order failures are recorded in,
/// and only transactions behind the held one are told to skip validation.
#[test]
fn earliest_bal_error_keeps_the_lowest_index() {
    let slot = EarliestBalError::default();
    assert!(!slot.precedes(usize::MAX));

    slot.record(7, EvmError::Custom("seven".into()));
    assert!(slot.precedes(8));
    assert!(!slot.precedes(7));
    assert!(!slot.precedes(3));

    slot.record(9, EvmError::Custom("nine".into()));
    slot.record(3, EvmError::Custom("three".into()));
    assert!(slot.precedes(4));
    assert!(!slot.precedes(3));

    assert_eq!(slot.take().map(|e| e.to_string()), Some("three".into()));
    assert!(slot.take().is_none());
    assert!(!slot.precedes(usize::MAX));
}
