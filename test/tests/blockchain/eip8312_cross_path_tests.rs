//! EIP-8312 cross-path differential: a block carrying UTXO traffic must import
//! identically down every execution path.
//!
//! EIP-8312 writes state from outside the EVM — the vault predeploy install at
//! activation and the block-end openings-root write — and each of those has to be
//! wired into the payload builder, the sequential importer, the pipeline importer
//! and the BAL-driven parallel importer in lockstep. A path that misses one, or
//! records it differently in the EIP-7928 access list, produces a different state
//! root for the same block.
//!
//! This is not hypothetical. The openings-root write shipped with a bug that made
//! the block unbuildable: it read the slot without registering the pre-value the
//! account-updates builder needs, so the block executed fine and then failed to
//! finalize. The unit tests asserted the slot's *value* and stopped, one layer
//! short of the failure, and every one of them passed. Only a live chain caught
//! it, and only through the consensus client's error — the execution client
//! logged nothing.
//!
//! The differential below is the coverage that was missing: build one block whose
//! transaction creates a UTXO, then import that same block into three independent
//! stores through three configurations, and require the state root, the access
//! list commitment and the receipts to agree — after first proving the block
//! actually did the EIP-8312 work, so agreement cannot be agreement on nothing.

use std::{fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER, Genesis,
        Receipt, Transaction, TxKind, block_access_list::BlockAccessList, ring_slot, utxo_vault,
    },
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

const TEST_MAX_FEE_PER_GAS: u64 = 10_000_000_000;
const TEST_GAS_LIMIT: u64 = 1_000_000;
/// Recipient of the UTXO the deposit creates. Deliberately an address with no
/// account: a UTXO recipient never gets a state leaf, which is the point of the
/// EIP, so this also pins that no path invents one.
const UTXO_RECIPIENT: Address = H160([0x9E; 20]);

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

/// A store on the Hegotá genesis with EIP-8312 active from block 0.
///
/// `utxoFramesTime` is injected rather than added to the fixture: the fixture is
/// shared with other tests, and EIP-8312 activating everywhere would change what
/// they execute.
async fn setup_store() -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-hegota.json"))
        .expect("open l1-hegota genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-hegota genesis");
    genesis.config.utxo_frames_time = Some(0);
    let chain_id = genesis.config.chain_id;
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
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

/// A vault deposit: value to `0x8312` with the 20-byte recipient as calldata.
fn deposit_tx(chain_id: u64, nonce: u64, recipient: Address, value: U256) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit: TEST_GAS_LIMIT,
        to: TxKind::Call(utxo_vault()),
        value,
        data: Bytes::copy_from_slice(recipient.as_bytes()),
        ..Default::default()
    })
}

/// Build the block under test: one deposit, on top of genesis.
async fn build_utxo_block() -> (Block, BlockAccessList, Vec<Receipt>) {
    let (store, chain_id) = setup_store().await;
    let blockchain = blockchain_with(store.clone(), false);
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    let signer: Signer = LocalSigner::new(test_secret_key()).into();
    let mut tx = deposit_tx(
        chain_id,
        0,
        UTXO_RECIPIENT,
        U256::from(10u64).pow(U256::from(18u64)),
    );
    tx.sign_inplace(&signer).await.unwrap();
    blockchain
        .add_transaction_to_pool(tx)
        .await
        .expect("deposit should enter the pool");

    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        // EIP-7843: an Amsterdam+ header must carry a slot number or header
        // validation rejects the block before anything under test runs.
        slot_number: Some(1),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let result = blockchain.build_payload(payload).unwrap();
    let bal = result
        .block_access_list
        .expect("an Amsterdam+ block must produce a BAL");
    (result.payload, bal, result.receipts)
}

/// The three import configurations under test, as (label, parallel, supply_bal).
///
/// `supply_bal` distinguishes the P2P-sync shape, where the caller hands the
/// block's access list to the importer and execution is driven from it, from the
/// local shape where the importer rebuilds it.
const PATHS: [(&str, bool, bool); 3] = [
    ("sequential", false, false),
    ("parallel", true, false),
    ("parallel+supplied-bal", true, true),
];

/// Import `block` into a fresh store and return the access list that path
/// produced plus the receipts it committed.
///
/// A successful return is itself the state-root assertion: the importer runs
/// `validate_state_root` and rejects any block whose computed root differs from
/// the header's, so a path that computed EIP-8312's out-of-EVM writes differently
/// cannot import this block at all. `a_forged_state_root_is_rejected_on_every_path`
/// pins that this check is live rather than assumed.
async fn import_and_snapshot(
    block: &Block,
    bal: &BlockAccessList,
    parallel: bool,
    supply_bal: bool,
) -> (Option<BlockAccessList>, Vec<Receipt>) {
    let (store, _) = setup_store().await;
    let blockchain = blockchain_with(store.clone(), parallel);
    let supplied = supply_bal.then(|| Arc::new(bal.clone()));
    let produced = blockchain
        .add_block_pipeline_bal(block.clone(), supplied)
        .expect("the block must import on every path");
    let receipts = store
        .get_receipts_for_block(&block.hash())
        .await
        .expect("imported block must have receipts");
    (produced, receipts)
}

#[tokio::test]
async fn a_utxo_bearing_block_imports_identically_on_every_path() {
    let (block, bal, build_receipts) = build_utxo_block().await;

    // ---- The block must actually have done EIP-8312 work -------------------
    // Without this the differential could pass by three paths agreeing that
    // nothing happened, which is the shape of test that missed the finalization
    // bug in the first place.
    assert!(
        build_receipts[0].succeeded,
        "the deposit must succeed, got: {build_receipts:?}"
    );
    assert_eq!(
        build_receipts[0]
            .logs
            .iter()
            .filter(|log| log.address == utxo_vault())
            .count(),
        1,
        "the deposit must emit exactly one UtxoCreated log from the vault"
    );

    // The block-end openings-root write must be in the built block's access list:
    // the parallel path derives state from the BAL alone, so an unrecorded
    // protocol write is a divergence there even when execution is correct.
    let ring = ring_slot(block.header.number);
    let vault_entry = bal
        .accounts()
        .iter()
        .find(|entry| entry.address == utxo_vault())
        .expect("the vault must appear in the block access list");
    assert!(
        vault_entry
            .storage_changes
            .iter()
            .any(|change| change.slot == ring),
        "the block-end openings-root write must be recorded in the BAL at slot {ring}"
    );

    // ---- The differential --------------------------------------------------
    let mut snapshots = Vec::new();
    for (label, parallel, supply) in PATHS {
        let (produced, receipts) = import_and_snapshot(&block, &bal, parallel, supply).await;
        snapshots.push((label, produced, receipts));
    }

    // Receipts must agree on every path, including the supplied-BAL one.
    let reference_receipts = &snapshots[0].2;
    for (label, _, receipts) in &snapshots[1..] {
        assert_eq!(
            receipts, reference_receipts,
            "receipts differ between {} and {label}",
            snapshots[0].0
        );
    }
    assert_eq!(
        reference_receipts.len(),
        1,
        "the block under test carries exactly one transaction"
    );

    // The rebuilt access list must agree between the paths that rebuild one. The
    // supplied-BAL path deliberately returns none: it drives execution from the
    // list the caller handed it instead of reconstructing one, so its agreement is
    // expressed by the import succeeding at all — that path still validates the
    // header's access-list commitment and state root against what it executed.
    let rebuilt: Vec<_> = snapshots
        .iter()
        .filter(|(_, produced, _)| produced.is_some())
        .collect();
    assert_eq!(
        rebuilt.len(),
        2,
        "exactly the two rebuilding paths should return an access list"
    );
    assert_eq!(
        rebuilt[0].1, rebuilt[1].1,
        "the rebuilt access list differs between {} and {}",
        rebuilt[0].0, rebuilt[1].0
    );
    assert!(
        snapshots
            .iter()
            .find(|(label, _, _)| *label == "parallel+supplied-bal")
            .expect("the supplied-BAL path must be exercised")
            .1
            .is_none(),
        "the supplied-BAL path is expected not to rebuild an access list"
    );

    // The rebuilt list must still carry the block-end openings-root write, so the
    // agreement above is agreement on a list that contains the EIP-8312 write
    // rather than on one that dropped it everywhere.
    assert!(
        rebuilt[0]
            .1
            .as_ref()
            .expect("rebuilt")
            .accounts()
            .iter()
            .find(|entry| entry.address == utxo_vault())
            .is_some_and(|entry| entry.storage_changes.iter().any(|c| c.slot == ring)),
        "the rebuilt access list must record the openings-root write at slot {ring}"
    );
}

#[tokio::test]
async fn a_forged_state_root_is_rejected_on_every_path() {
    // The negative control for the differential above. Every path imports the
    // real block, so they agree; this shows that agreement is enforced rather
    // than assumed, by flipping one bit of the state root and requiring each path
    // to reject it. Without this, a path that skipped state-root validation
    // entirely would look identical to one that agreed.
    let (block, bal, _) = build_utxo_block().await;

    for (label, parallel, supply) in PATHS {
        let mut forged = block.clone();
        let mut root = forged.header.state_root.0;
        root[31] ^= 0x01;
        forged.header.state_root = H256(root);

        let (store, _) = setup_store().await;
        let blockchain = blockchain_with(store, parallel);
        let supplied = supply.then(|| Arc::new(bal.clone()));
        let result = blockchain.add_block_pipeline_bal(forged, supplied);
        assert!(
            result.is_err(),
            "a forged state root must be rejected on the {label} path"
        );
    }
}

#[tokio::test]
async fn a_utxo_recipient_gets_no_account_leaf_on_any_path() {
    // The EIP's state claim, against committed state rather than a VM snapshot:
    // the paid address must not exist after the block is imported, on every path,
    // and the value must sit in the vault instead. A path that credited the
    // recipient directly would be self-consistent and so would not show up as a
    // root mismatch — it needs asserting separately.
    let (block, bal, _) = build_utxo_block().await;

    for (label, parallel, supply) in PATHS {
        let (store, _) = setup_store().await;
        let blockchain = blockchain_with(store.clone(), parallel);
        let supplied = supply.then(|| Arc::new(bal.clone()));
        blockchain
            .add_block_pipeline_bal(block.clone(), supplied)
            .expect("import");

        let root = block.header.state_root;
        let recipient = store
            .get_account_state_by_root(root, UTXO_RECIPIENT)
            .expect("read recipient state");
        assert!(
            recipient.is_none_or(|acc| acc.balance.is_zero() && acc.nonce == 0),
            "a UTXO recipient must have no account leaf ({label})"
        );

        let vault = store
            .get_account_state_by_root(root, utxo_vault())
            .expect("read vault state")
            .expect("the vault must exist once EIP-8312 is active");
        assert!(
            !vault.balance.is_zero(),
            "the vault must custody the deposited value ({label})"
        );
    }
}
