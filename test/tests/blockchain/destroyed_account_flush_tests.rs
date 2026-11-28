//! Blocks that destroy an account and re-create it, imported down the sequential path,
//! whose streaming flushes every few transactions split the block into batches.
//!
//! Three shapes:
//!
//! - a transaction CREATE2s P and self-destructs it (EIP-6780), and a later one in the
//!   same batch re-creates P, writes a slot the first never read, and fails, so the
//!   revert leaves the slot's zero on the `Destroyed` P;
//! - the same with P funded in an earlier block and a flush between the two
//!   transactions, the re-creation writing the slot the first wrote;
//! - P re-created for good, its slot committed by a flush, then cleared by a call in a
//!   later batch, a deletion the merkleizer must see.
//!
//! Every block is built by ethrex's own builder, whose single-pass execution has no
//! flushes, so its state root is the reference the streaming import must reproduce.

use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER,
        Genesis, GenesisAccount, Transaction, TxKind,
    },
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_levm::utils::calculate_create2_address;
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

const MAX_FEE_PER_GAS: u64 = 10_000_000_000;
const TX_GAS_LIMIT: u64 = 300_000;

/// `CALLDATASIZE PUSH1 0 PUSH1 0 CALLDATACOPY PUSH1 0 CALLDATASIZE PUSH1 0 PUSH1 0 CREATE2
/// PUSH1 22 JUMPI PUSH1 0 PUSH1 0 REVERT JUMPDEST STOP`: CREATE2s the calldata as init code
/// with salt 0, and reverts when the creation failed, so the whole transaction fails as
/// in the report.
const FACTORY: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xfa, 0xc7,
]);
const FACTORY_CODE: [u8; 24] = [
    0x36, 0x60, 0x00, 0x60, 0x00, 0x37, 0x60, 0x00, 0x36, 0x60, 0x00, 0x60, 0x00, 0xf5, 0x60, 22,
    0x57, 0x60, 0x00, 0x60, 0x00, 0xfd, 0x5b, 0x00,
];
/// Receives the self-destructed balance.
const BENEFICIARY: Address = H160([0xbe; 20]);
/// The init code branches on this account's balance, so one init code (hence one CREATE2
/// address) first destroys P and, once this account is funded, re-creates it.
const MODE: Address = H160([0x0d; 20]);

/// Init code: `PUSH20 MODE BALANCE PUSH1 52 JUMPI` then the destroying branch
/// `PUSH1 1 PUSH1 4 SSTORE PUSH20 BENEFICIARY SELFDESTRUCT`, then `JUMPDEST` and `tail`.
fn init_code(tail: &[u8]) -> Bytes {
    let mut code = vec![0x73];
    code.extend_from_slice(MODE.as_bytes());
    code.extend_from_slice(&[0x31, 0x60, 52, 0x57]);
    code.extend_from_slice(&[0x60, 0x01, 0x60, 0x04, 0x55, 0x73]);
    code.extend_from_slice(BENEFICIARY.as_bytes());
    code.push(0xff);
    assert_eq!(code.len(), 52);
    code.push(0x5b);
    code.extend_from_slice(tail);
    Bytes::from(code)
}

/// Re-creation that writes slot `slot` and reverts, failing its transaction.
fn reverting_init(slot: u8) -> Bytes {
    init_code(&[0x60, 0x01, 0x60, slot, 0x55, 0x60, 0x00, 0x60, 0x00, 0xfd])
}

/// Re-creation that writes slot 4 to 7 and deploys `PUSH1 0 PUSH1 4 SSTORE STOP`, which
/// clears the slot when called.
fn recreating_init() -> Bytes {
    init_code(&[
        0x60, 0x07, 0x60, 0x04, 0x55, // SSTORE(4, 7)
        0x65, 0x60, 0x00, 0x60, 0x04, 0x55, 0x00, // PUSH6 runtime
        0x60, 0x00, 0x52, // MSTORE(0, runtime)
        0x60, 0x06, 0x60, 0x1a, 0xf3, // RETURN(26, 6)
    ])
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// One sender per transaction, each at nonce 0.
fn sender_key(index: usize) -> SecretKey {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&(index as u64 + 1).to_be_bytes());
    SecretKey::from_slice(&bytes).expect("small scalars are valid secret keys")
}

/// An Osaka store: the factory in the genesis, and `senders` funded.
async fn setup_store(senders: &[Address]) -> Store {
    let file =
        File::open(workspace_root().join("fixtures/genesis/l1.json")).expect("open l1 genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1 genesis");
    assert!(
        genesis.config.amsterdam_time.is_none(),
        "the sequential path is the pre-Amsterdam one"
    );
    genesis.alloc.insert(
        FACTORY,
        GenesisAccount {
            code: Bytes::copy_from_slice(&FACTORY_CODE),
            storage: Default::default(),
            balance: U256::zero(),
            nonce: 1,
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
    let mut store = Store::new("store.db", EngineType::InMemory).expect("in-memory store");
    store.add_initial_state(genesis).await.expect("genesis");
    store
}

/// What each transaction of a block does.
#[derive(Clone)]
enum Step {
    /// A plain transfer from its sender to itself.
    Filler,
    /// Funds `MODE`, flipping the init code to its re-creating branch.
    FlipMode,
    /// Sends 1 wei to `to`.
    Fund(Address),
    /// Has the factory CREATE2 `init`.
    Create(Bytes),
    /// Calls P with no data.
    Call(Address),
}

async fn transaction(chain_id: u64, signer: &Signer, step: &Step) -> Transaction {
    let (to, value, data) = match step {
        Step::Filler => (TxKind::Call(signer.address()), U256::zero(), Bytes::new()),
        Step::FlipMode => (TxKind::Call(MODE), U256::one(), Bytes::new()),
        Step::Fund(to) => (TxKind::Call(*to), U256::one(), Bytes::new()),
        Step::Create(init) => (TxKind::Call(FACTORY), U256::zero(), init.clone()),
        Step::Call(to) => (TxKind::Call(*to), U256::zero(), Bytes::new()),
    };
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        gas_limit: TX_GAS_LIMIT,
        to,
        value,
        data,
        ..Default::default()
    });
    tx.sign_inplace(signer).await.unwrap();
    tx
}

/// Builds the block of `steps` on `parent` with ethrex's builder, asserting every
/// transaction was included.
async fn build_block(
    store: &Store,
    parent: &BlockHeader,
    signers: &[Signer],
    steps: &[Step],
) -> Block {
    let chain_id = store.get_chain_config().chain_id;
    let mut txs = Vec::with_capacity(steps.len());
    for (signer, step) in signers.iter().zip(steps) {
        txs.push(transaction(chain_id, signer, step).await);
    }
    let blockchain = Blockchain::new(store.clone(), BlockchainOptions::default());
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, store, Bytes::new()).unwrap();
    let result = blockchain
        .build_payload_with_transactions(payload, txs)
        .expect("the block builds");
    assert_eq!(
        result.payload.body.transactions.len(),
        steps.len(),
        "every transaction is included"
    );
    result.payload
}

/// Builds the blocks of `blocks` (one list of steps each, applied in order on a builder
/// store) and imports them down the sequential path on a fresh store.
async fn import_sequentially(blocks: &[Vec<Step>]) -> Result<(), String> {
    let total: usize = blocks.iter().map(Vec::len).sum();
    let signers: Vec<Signer> = (0..total)
        .map(|i| LocalSigner::new(sender_key(i)).into())
        .collect();
    let senders: Vec<Address> = signers.iter().map(Signer::address).collect();

    let build_store = setup_store(&senders).await;
    let builder = Blockchain::new(build_store.clone(), BlockchainOptions::default());
    let mut parent = build_store.get_block_header(0).unwrap().unwrap();
    let mut built = Vec::with_capacity(blocks.len());
    let mut next_signer = 0;
    for steps in blocks {
        let block_signers = &signers[next_signer..next_signer + steps.len()];
        next_signer += steps.len();
        let block = build_block(&build_store, &parent, block_signers, steps).await;
        builder
            .add_block(block.clone())
            .expect("the builder's own block imports on its store");
        parent = block.header.clone();
        built.push(block);
    }

    let blockchain = Blockchain::new(
        setup_store(&senders).await,
        BlockchainOptions {
            bal_parallel_exec_enabled: false,
            ..Default::default()
        },
    );
    for block in built {
        blockchain
            .add_block_pipeline(block, None)
            .map_err(|e| format!("{e:?}"))?;
    }
    Ok(())
}

fn p_of(init: &Bytes) -> Address {
    calculate_create2_address(FACTORY, init, U256::zero()).expect("create2 address")
}

/// P is created, destroyed, then re-created with a write to a slot the first creation
/// never read, and that re-creation fails: all in one batch.
#[tokio::test]
async fn reverted_recreation_writing_a_fresh_slot_in_one_batch() {
    let init = reverting_init(5);
    import_sequentially(&[vec![
        Step::Create(init.clone()),
        Step::FlipMode,
        Step::Create(init),
    ]])
    .await
    .expect("a valid block must be imported");
}

/// P, funded in an earlier block, is destroyed by the fifth transaction, after which the
/// first flush runs, and re-created by a failing transaction that writes the slot the
/// destroying creation wrote.
#[tokio::test]
async fn reverted_recreation_across_a_flush_of_a_funded_account() {
    let init = reverting_init(4);
    let p = p_of(&init);
    import_sequentially(&[
        vec![Step::Fund(p)],
        vec![
            Step::Filler,
            Step::Filler,
            Step::Filler,
            Step::Filler,
            Step::Create(init.clone()),
            Step::FlipMode,
            Step::Create(init),
            Step::Filler,
        ],
    ])
    .await
    .expect("a valid block must be imported");
}

/// P is created and destroyed, re-created with slot 4 set to 7, and, after the first
/// flush has committed that slot, called to clear it.
#[tokio::test]
async fn slot_cleared_after_a_flush_on_a_recreated_account() {
    let init = recreating_init();
    let p = p_of(&init);
    import_sequentially(&[vec![
        Step::Create(init.clone()),
        Step::FlipMode,
        Step::Create(init),
        Step::Filler,
        Step::Filler,
        Step::Call(p),
    ]])
    .await
    .expect("a valid block must be imported");
}
