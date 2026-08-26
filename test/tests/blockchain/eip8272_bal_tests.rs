//! EIP-8272 × EIP-7928: how recent-root traffic lands in the block access list.
//!
//! The two sides of the EIP touch recent-root storage in opposite ways and the
//! access list has to tell them apart. A write through the predeploy is an
//! ordinary storage change under the writing transaction's index. A reference
//! carried by a frame transaction is checked against the transaction pre-state
//! before any frame runs, and is only ever a *read* — recording it as a change
//! would make the BAL-driven parallel importer reconstruct a post-state that
//! overwrites the entry with itself under the wrong index.
//!
//! One block carries both, so the distinction is exercised where it matters:
//! against the real builder, and then again against the importer, which must
//! reconstruct the same commitment from the same block.

use std::{collections::BTreeMap, fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER, Frame,
        FrameMode, FrameTransaction, Genesis, GenesisAccount, RecentRootReference, Transaction,
        TxKind, block_access_list::BlockAccessList, frame_tx_recent_root,
    },
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::system_contracts::RECENT_ROOT_RUNTIME_BYTECODE;
use secp256k1::SecretKey;

const TEST_MAX_FEE_PER_GAS: u64 = 10_000_000_000;
const TEST_GAS_LIMIT: u64 = 1_000_000;
/// The frame transaction's sender. Its code approves both scopes, so the
/// transaction carries no outer signature and these tests stay on the access
/// list rather than on EIP-8141 signature handling.
const FRAME_SENDER: Address = H160([0xAB; 20]);
/// `PUSH1 3; PUSH1 0; PUSH1 0; APPROVE; STOP` — APPROVE_EXECUTION_AND_PAYMENT.
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA, 0x00];

/// The block under test is built at this slot. The pre-existing entry is one
/// slot older, the closest a reference may be (§Current slot: "References MUST
/// target slots strictly before `current_slot`").
const BLOCK_SLOT: u64 = 1;
const REFERENCED_SLOT: u64 = BLOCK_SLOT - 1;

/// `salt ‖ root` the EIP-1559 transaction writes.
const WRITE_SALT: [u8; 32] = [0x11; 32];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn test_secret_key() -> SecretKey {
    // The l1-hegota genesis funds this key's address.
    SecretKey::from_slice(
        &hex::decode("941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e").unwrap(),
    )
    .unwrap()
}

fn writer_address() -> Address {
    let signer: Signer = LocalSigner::new(test_secret_key()).into();
    signer.address()
}

/// `source_id = keccak256(source_address ‖ salt)` over the unpadded 20-byte
/// address and the 32-byte salt.
fn source_id(caller: Address, salt: &[u8; 32]) -> H256 {
    let mut pre = [0u8; 52];
    pre[..20].copy_from_slice(caller.as_bytes());
    pre[20..52].copy_from_slice(salt);
    H256(ethrex_crypto::keccak::keccak_hash(pre))
}

/// The entry the reference-carrying frame transaction points at: already
/// committed in the parent state, one slot before the block being built.
fn referenced_entry() -> RecentRootReference {
    RecentRootReference {
        source_id: source_id(FRAME_SENDER, &[0x99; 32]),
        slot: REFERENCED_SLOT,
        root: H256::repeat_byte(0xEE),
    }
}

/// The entry the EIP-1559 transaction commits by calling the predeploy.
fn written_entry() -> RecentRootReference {
    RecentRootReference {
        source_id: source_id(writer_address(), &WRITE_SALT),
        slot: BLOCK_SLOT,
        root: H256::repeat_byte(0x22),
    }
}

/// A store on the Hegotá genesis with the predeploy already carrying its code
/// and one committed entry, plus a frame-transaction sender.
///
/// The predeploy is seeded with code *and* storage together: EIP-8272
/// §Activation makes a payload invalid when the address has storage but no
/// code, and the installer enforces that.
async fn setup_store(store_name: &str) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-hegota.json"))
        .expect("open l1-hegota genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-hegota genesis");

    let entry = referenced_entry();
    genesis.alloc.insert(
        frame_tx_recent_root(),
        GenesisAccount {
            code: Bytes::from_static(&RECENT_ROOT_RUNTIME_BYTECODE),
            storage: BTreeMap::from([(
                U256::from_big_endian(entry.storage_key().as_bytes()),
                U256::from_big_endian(entry.entry_hash().as_bytes()),
            )]),
            balance: U256::zero(),
            nonce: 1,
        },
    );
    genesis.alloc.insert(
        FRAME_SENDER,
        GenesisAccount {
            code: Bytes::from_static(APPROVE_BOTH_CODE),
            storage: BTreeMap::new(),
            balance: U256::from(10u64).pow(U256::from(20u64)),
            nonce: 0,
        },
    );

    let chain_id = genesis.config.chain_id;
    let mut store = Store::new(store_name, EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    (store, chain_id)
}

fn blockchain_with(store: Store, bal_parallel_exec_enabled: bool) -> Blockchain {
    Blockchain::new(
        store,
        BlockchainOptions {
            bal_parallel_exec_enabled,
            ..Default::default()
        },
    )
}

/// A plain EOA call to the predeploy with `salt ‖ root` as calldata.
fn write_tx(chain_id: u64) -> Transaction {
    let entry = written_entry();
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit: TEST_GAS_LIMIT,
        to: TxKind::Call(frame_tx_recent_root()),
        value: U256::zero(),
        data: Bytes::from([WRITE_SALT.as_slice(), entry.root.as_bytes()].concat()),
        ..Default::default()
    })
}

/// A frame transaction declaring the pre-existing entry. One self-verifying
/// frame is enough: the reference check runs before any frame does.
fn reference_tx(chain_id: u64) -> Transaction {
    Transaction::FrameTransaction(FrameTransaction {
        chain_id,
        // A non-vault sender must select 1..=16 nonce keys; key 0 is the
        // sender's linear account nonce.
        nonce_keys: vec![U256::zero()],
        nonce_seq: 0,
        sender: FRAME_SENDER,
        frames: vec![Frame {
            mode: FrameMode::Verify as u8,
            flags: 0x03,
            target: Some(FRAME_SENDER),
            gas_limit: 100_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        }],
        signatures: vec![],
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        recent_root_references: vec![referenced_entry()],
        ..Default::default()
    })
}

/// Build the block under test: the predeploy write first, then the
/// reference-carrying frame transaction.
async fn build_block() -> (Block, BlockAccessList) {
    let (store, chain_id) = setup_store("eip8272-bal-build").await;
    let blockchain = blockchain_with(store.clone(), false);
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    let signer: Signer = LocalSigner::new(test_secret_key()).into();
    let mut write = write_tx(chain_id);
    write.sign_inplace(&signer).await.unwrap();

    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(BLOCK_SLOT),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        inclusion_list_transactions: None,
    };
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let result = blockchain
        .build_payload_with_transactions(payload, vec![write, reference_tx(chain_id)])
        .expect("a block carrying a recent-root write and a reference must be buildable");
    let bal = result
        .block_access_list
        .expect("an Amsterdam+ block must produce a BAL");

    assert_eq!(
        result.payload.body.transactions.len(),
        2,
        "both transactions must be included; receipts={:?}",
        result.receipts
    );
    assert!(
        result.receipts.iter().all(|r| r.succeeded),
        "both transactions must succeed; receipts={:?}",
        result.receipts
    );
    (result.payload, bal)
}

fn predeploy_changes(
    bal: &BlockAccessList,
) -> &ethrex_common::types::block_access_list::AccountChanges {
    bal.accounts()
        .iter()
        .find(|entry| entry.address == frame_tx_recent_root())
        .expect("RECENT_ROOT_ADDRESS must appear in the block access list")
}

#[tokio::test]
async fn a_write_is_a_change_and_a_reference_is_a_read() {
    let (_, bal) = build_block().await;
    let changes = predeploy_changes(&bal);

    let written = U256::from_big_endian(written_entry().storage_key().as_bytes());
    let referenced = U256::from_big_endian(referenced_entry().storage_key().as_bytes());

    // The write, under the writing transaction's index. EIP-7928 numbers
    // transactions from 1; index 0 is the pre-execution phase.
    let slot_change = changes
        .storage_changes
        .iter()
        .find(|change| change.slot == written)
        .expect("the predeploy write must be recorded as a storage change");
    assert_eq!(
        slot_change
            .slot_changes
            .iter()
            .map(|c| c.block_access_index)
            .collect::<Vec<_>>(),
        vec![1],
        "the write must be attributed to the first transaction"
    );
    assert_eq!(
        slot_change.slot_changes[0].post_value,
        U256::from_big_endian(written_entry().entry_hash().as_bytes()),
        "the recorded post-value must be the committed entry hash"
    );

    // The reference, as a read and nothing else. The check runs against the
    // transaction pre-state and never writes, so a change here would be a
    // post-state the parallel importer would then reproduce.
    assert!(
        changes.storage_reads.contains(&referenced),
        "the reference's storage key must be recorded as a read"
    );
    assert!(
        changes
            .storage_changes
            .iter()
            .all(|change| change.slot != referenced),
        "the reference's storage key must never be recorded as a change"
    );
}

#[tokio::test]
async fn the_access_list_commitment_survives_a_rebuild() {
    // The builder commits `blockAccessListHash` in the header; the importer
    // recomputes it from its own execution and rejects the block on a mismatch.
    // A write or a reference recorded differently on the two sides is a chain
    // split with no state-root disagreement to point at it.
    let (block, build_bal) = build_block().await;

    for parallel in [false, true] {
        let (store, _) = setup_store(&format!("eip8272-bal-import-{parallel}")).await;
        let blockchain = blockchain_with(store, parallel);
        let rebuilt = blockchain
            .add_block_pipeline_bal(block.clone(), None)
            .expect("the block must import")
            .expect("a rebuilding path must return an access list");

        assert_eq!(
            rebuilt, build_bal,
            "the rebuilt access list differs from the built one (parallel={parallel})"
        );
    }

    // Agreement on a list that still contains both entries, rather than on one
    // that dropped them everywhere.
    let changes = predeploy_changes(&build_bal);
    assert!(!changes.storage_changes.is_empty() && !changes.storage_reads.is_empty());
    assert_eq!(
        block.header.block_access_list_hash,
        Some(build_bal.compute_hash(&ethrex_crypto::NativeCrypto)),
        "the header must commit to the access list under test"
    );
}

// ==================== EIP-7805 IL-first sequencing ====================
//
// `apply_inclusion_list_transactions` sequences the inclusion list ahead of the
// mempool and assigns each entry `context.payload.body.transactions.len() + 1`
// as its EIP-7928 block access index, on a code path separate from
// `fill_transactions`. If the two disagree about indices, the builder and the
// importer commit different `blockAccessListHash` values for the same block —
// a chain split with no state-root mismatch to point at it, which is the
// failure mode FOCIL adds on top of everything else.

/// Build a block whose inclusion list carries the predeploy write, leaving the
/// reference-carrying frame transaction to arrive from the mempool behind it.
async fn build_il_sequenced_block() -> (Block, BlockAccessList) {
    let (store, chain_id) = setup_store("eip8272-bal-il").await;
    let blockchain = blockchain_with(store.clone(), false);
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    let signer: Signer = LocalSigner::new(test_secret_key()).into();
    let mut write = write_tx(chain_id);
    write.sign_inplace(&signer).await.unwrap();
    let il = vec![write.clone()];

    // The frame transaction goes through the pool, so the builder picks it up
    // in `fill_transactions` after the list has been sequenced.
    blockchain
        .add_transaction_to_pool(reference_tx(chain_id))
        .await
        .expect("the reference-carrying frame transaction must enter the pool");

    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(BLOCK_SLOT),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        inclusion_list_transactions: Some(il.clone()),
    };
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let result = blockchain
        .build_payload_with_il(payload, &il)
        .expect("a block sequencing an inclusion list ahead of a frame tx must be buildable");
    let bal = result
        .block_access_list
        .expect("an Amsterdam+ block must produce a BAL");

    assert_eq!(
        result.payload.body.transactions.len(),
        2,
        "the listed write and the pooled frame transaction must both be included; receipts={:?}",
        result.receipts
    );
    assert_eq!(
        result.payload.body.transactions[0].hash(&ethrex_crypto::NativeCrypto),
        write.hash(&ethrex_crypto::NativeCrypto),
        "the inclusion-list entry must be sequenced first"
    );
    assert!(
        result.receipts.iter().all(|r| r.succeeded),
        "both transactions must succeed; receipts={:?}",
        result.receipts
    );
    (result.payload, bal)
}

#[tokio::test]
async fn an_il_sequenced_write_is_indexed_ahead_of_the_pooled_frame_transaction() {
    let (_, bal) = build_il_sequenced_block().await;
    let changes = predeploy_changes(&bal);

    let written = U256::from_big_endian(written_entry().storage_key().as_bytes());
    let slot_change = changes
        .storage_changes
        .iter()
        .find(|change| change.slot == written)
        .expect("the listed write must be recorded as a storage change");
    assert_eq!(
        slot_change
            .slot_changes
            .iter()
            .map(|c| c.block_access_index)
            .collect::<Vec<_>>(),
        vec![1],
        "an inclusion-list entry takes index 1, the same index fill_transactions \
         would have given the block's first transaction"
    );

    // The frame transaction still reads its reference, so the block genuinely
    // exercises both paths rather than one of them silently dropping out.
    assert!(
        changes.storage_reads.contains(&U256::from_big_endian(
            referenced_entry().storage_key().as_bytes()
        )),
        "the pooled frame transaction's reference must still be recorded as a read"
    );
}

#[tokio::test]
async fn il_sequencing_produces_the_same_commitment_on_re_import() {
    // The importer knows nothing about inclusion lists: it walks the block's
    // transactions in order and indexes them from 1. The builder reached the
    // same block down two code paths, so the two must agree — otherwise every
    // block carrying an inclusion list is rejected by its own network.
    let (block, build_bal) = build_il_sequenced_block().await;

    for parallel in [false, true] {
        let (store, _) = setup_store(&format!("eip8272-bal-il-import-{parallel}")).await;
        let blockchain = blockchain_with(store, parallel);
        let rebuilt = blockchain
            .add_block_pipeline_bal(block.clone(), None)
            .expect("the block must import")
            .expect("a rebuilding path must return an access list");

        assert_eq!(
            rebuilt, build_bal,
            "the rebuilt access list differs from the IL-sequenced one (parallel={parallel})"
        );
    }

    assert_eq!(
        block.header.block_access_list_hash,
        Some(build_bal.compute_hash(&ethrex_crypto::NativeCrypto)),
        "the header must commit to the access list under test"
    );
}
