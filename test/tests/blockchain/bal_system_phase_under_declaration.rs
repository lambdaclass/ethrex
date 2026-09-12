//! Regression tests for issue #7109: the default parallel block-import path
//! used to accept a block whose BAL omits an account or storage slot touched
//! outside transaction execution, in the pre-block system calls (EIP-2935
//! history, EIP-4788 beacon roots) or the post-block request and withdrawal
//! phase (EIP-7002, EIP-7251, EIP-8282).
//!
//! Transaction reads are covered by the per-tx shadow recorder. Those phases
//! only had the `unread_storage_reads` / `unaccessed_pure_accounts` checklists,
//! which are built FROM the supplied BAL and so catch over-declaration only:
//! removing an entry just shortens them.
//!
//! Trimming an entry that carries no writes moves neither `state_root` nor gas,
//! and the header commitment is recomputed over the trimmed BAL, so the block
//! stays self-consistent. Only regenerate-and-compare sees it, which is what
//! geth, reth, erigon and our own sequential path do.
//!
//! Each test builds a valid Amsterdam block, trims one entry, recomputes the
//! commitment, and asserts the parallel path now rejects it for the right
//! reason, against a positive control that imports the block untrimmed.

use std::{fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Genesis, Withdrawal,
        block_access_list::BlockAccessList,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::system_contracts::BEACON_ROOTS_ADDRESS;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn setup_store() -> Store {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-bal.json"))
        .expect("open l1-bal genesis");
    let genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-bal genesis");
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    store
}

/// Produce a fully-valid empty Amsterdam block on top of genesis and the
/// canonical BAL the producer recorded for it.
async fn build_valid_amsterdam_block(store: &Store) -> (Block, BlockAccessList) {
    build_amsterdam_block_with_withdrawals(store, Vec::new()).await
}

/// As above, with an explicit withdrawal list.
async fn build_amsterdam_block_with_withdrawals(
    store: &Store,
    withdrawals: Vec<Withdrawal>,
) -> (Block, BlockAccessList) {
    let bc = Blockchain::new(store.clone(), BlockchainOptions::default());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(withdrawals),
        beacon_root: Some(H256::zero()),
        slot_number: Some(1),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, store, Bytes::new()).unwrap();
    let result = bc.build_payload(payload).unwrap();
    let bal = result
        .block_access_list
        .expect("amsterdam block must produce a BAL");
    (result.payload, bal)
}

/// Index of the first entry that only records reads: no storage writes, no
/// balance / nonce / code changes. Removing one of these leaves `state_root`
/// and gas untouched.
fn first_read_only(bal: &BlockAccessList) -> usize {
    bal.accounts()
        .iter()
        .position(|a| {
            a.storage_changes.is_empty()
                && a.balance_changes.is_empty()
                && a.nonce_changes.is_empty()
                && a.code_changes.is_empty()
                && !a.storage_reads.is_empty()
        })
        .expect("block should record at least one read-only account")
}

#[tokio::test]
async fn parallel_path_rejects_bal_missing_a_system_phase_read() {
    let build_store = setup_store().await;
    let (mut block, bal) = build_valid_amsterdam_block(&build_store).await;

    let victim = first_read_only(&bal);
    let dropped = bal.accounts()[victim].clone();

    let mut kept = bal.accounts().to_vec();
    kept.remove(victim);
    let trimmed = Arc::new(BlockAccessList::from_accounts(kept));

    // Self-consistent: the commitment is recomputed over the trimmed BAL, so a
    // hash check cannot catch this.
    let trimmed_hash = trimmed.compute_hash(&NativeCrypto);
    assert_ne!(trimmed_hash, bal.compute_hash(&NativeCrypto));
    block.header.block_access_list_hash = Some(trimmed_hash);

    // Positive control: the untouched block still imports on the parallel path,
    // so a rejection below is about the trim and not about the harness.
    let control_store = setup_store().await;
    let control_bc = Blockchain::new(
        control_store,
        BlockchainOptions {
            bal_parallel_exec_enabled: true,
            ..Default::default()
        },
    );
    let (control_block, control_bal) = build_valid_amsterdam_block(&build_store).await;
    let control = control_bc.add_block_pipeline_bal(control_block, Some(Arc::new(control_bal)));
    assert!(
        control.is_ok(),
        "parallel path must accept the unmodified block, got: {control:?}"
    );

    // SEQUENTIAL: regenerates the canonical BAL and byte-compares, so it sees
    // the missing entry.
    let seq_store = setup_store().await;
    let seq_bc = Blockchain::new(
        seq_store,
        BlockchainOptions {
            bal_parallel_exec_enabled: false,
            ..Default::default()
        },
    );
    let seq = seq_bc.add_block_pipeline_bal(block.clone(), Some(trimmed.clone()));
    assert!(
        seq.is_err(),
        "sequential path must reject a BAL missing {:?}, got: {seq:?}",
        dropped.address
    );

    // PARALLEL (the default): same block, same trimmed BAL, only the flag
    // flipped. This is the finding — it accepts.
    let par_store = setup_store().await;
    let par_bc = Blockchain::new(
        par_store,
        BlockchainOptions {
            bal_parallel_exec_enabled: true,
            ..Default::default()
        },
    );
    let par = par_bc.add_block_pipeline_bal(block, Some(trimmed));
    let err = par.expect_err(&format!(
        "parallel path accepted a block whose BAL omits {:?} ({} storage_reads), \
         which the sequential path and geth/reth/erigon all reject",
        dropped.address,
        dropped.storage_reads.len()
    ));
    let msg = err.to_string();
    assert!(
        msg.contains("BAL validation failed") && msg.contains("missing from BAL"),
        "rejected for the wrong reason: {msg}"
    );
}

/// The pre-block system calls are the other half of the gap: their reads land at
/// `block_access_index = 0`, and dropping a `storage_reads` slot from an account
/// that also writes leaves `state_root` untouched, so only a completeness check
/// sees it. Pinned to the beacon-roots contract so the test cannot silently drift
/// onto a post-block request predeploy and start proving the other call site.
#[tokio::test]
async fn parallel_path_rejects_bal_missing_a_pre_block_system_call_read() {
    let build_store = setup_store().await;
    let (mut block, bal) = build_valid_amsterdam_block(&build_store).await;

    let address = BEACON_ROOTS_ADDRESS.address;
    let victim = bal
        .accounts()
        .iter()
        .position(|a| a.address == address)
        .expect("block should record the beacon-roots contract");
    assert!(
        !bal.accounts()[victim].storage_changes.is_empty()
            && !bal.accounts()[victim].storage_reads.is_empty(),
        "beacon-roots entry must have both writes and reads for this trim to be read-only"
    );

    let mut kept = bal.accounts().to_vec();
    kept[victim].storage_reads.clear();
    let trimmed = Arc::new(BlockAccessList::from_accounts(kept));
    block.header.block_access_list_hash = Some(trimmed.compute_hash(&NativeCrypto));

    let par_store = setup_store().await;
    let par_bc = Blockchain::new(
        par_store,
        BlockchainOptions {
            bal_parallel_exec_enabled: true,
            ..Default::default()
        },
    );
    let par = par_bc.add_block_pipeline_bal(block, Some(trimmed));
    let err = par.expect_err(&format!(
        "parallel path accepted a block whose BAL drops the storage_reads of {address:?}"
    ));
    let msg = err.to_string();
    assert!(
        msg.contains("BAL validation failed") && msg.contains("missing from BAL"),
        "rejected for the wrong reason: {msg}"
    );
}

/// Withdrawal recipients must appear in the BAL "regardless of amount"
/// (EIP-7928), but `process_withdrawals` only loads the account when
/// `amount > 0`. A 0-amount recipient therefore never enters
/// `current_accounts_state`, so walking that cache cannot see it and the
/// membership check has to come from the withdrawal list itself.
#[tokio::test]
async fn parallel_path_rejects_bal_missing_a_zero_amount_withdrawal_recipient() {
    let recipient = H160::from_low_u64_be(0xdeadbeef);
    let withdrawals = vec![Withdrawal {
        index: 0,
        validator_index: 0,
        address: recipient,
        amount: 0,
    }];

    let build_store = setup_store().await;
    let (mut block, bal) =
        build_amsterdam_block_with_withdrawals(&build_store, withdrawals.clone()).await;

    // The producer records the recipient even though the withdrawal moves no
    // value, so the canonical BAL carries a pure-access entry for it.
    let victim = bal
        .accounts()
        .iter()
        .position(|a| a.address == recipient)
        .expect("canonical BAL must record a 0-amount withdrawal recipient");

    // Positive control: unmodified, this block imports on the parallel path.
    let control_store = setup_store().await;
    let control_bc = Blockchain::new(
        control_store,
        BlockchainOptions {
            bal_parallel_exec_enabled: true,
            ..Default::default()
        },
    );
    let (control_block, control_bal) =
        build_amsterdam_block_with_withdrawals(&build_store, withdrawals).await;
    let control = control_bc.add_block_pipeline_bal(control_block, Some(Arc::new(control_bal)));
    assert!(
        control.is_ok(),
        "parallel path must accept the unmodified block, got: {control:?}"
    );

    // Drop the recipient. Its entry has no changes and no reads, so this moves
    // neither state_root nor gas.
    let mut kept = bal.accounts().to_vec();
    kept.remove(victim);
    let trimmed = Arc::new(BlockAccessList::from_accounts(kept));
    block.header.block_access_list_hash = Some(trimmed.compute_hash(&NativeCrypto));

    let par_store = setup_store().await;
    let par_bc = Blockchain::new(
        par_store,
        BlockchainOptions {
            bal_parallel_exec_enabled: true,
            ..Default::default()
        },
    );
    let par = par_bc.add_block_pipeline_bal(block, Some(trimmed));
    let err = par.expect_err(
        "parallel path accepted a block whose BAL omits a 0-amount withdrawal recipient",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("withdrawal recipient") && msg.contains("missing from BAL"),
        "rejected for the wrong reason: {msg}"
    );
}
