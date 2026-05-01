//! Builder / validator parity tests for Amsterdam (EIP-7928 + EIP-8037).
//!
//! # Why this module exists
//!
//! When ethrex produces a block as a builder and that same block is later
//! executed by ethrex as a validator (or by any EELS-compatible validator),
//! both code paths **must** reach bit-identical conclusions on:
//!
//! - the final state root,
//! - the receipts root,
//! - the block-level gas accounting (`max(block_regular_gas_used, block_state_gas_used)`),
//! - the contents and hash of the Block Access List.
//!
//! If they disagree, the builder-produced block will be rejected at inclusion,
//! the slot is missed, and the validator loses its proposer reward. This is a
//! correctness-critical class of bug: it can only be triggered against live
//! traffic, it's silent in any single-path test, and a single missed slot can
//! costs more than a regression test ever will.
//!
//! The Amsterdam rollup (EIP-7928 Block Access Lists, EIP-8037 two-dimensional
//! state gas, EIP-7976/7981 calldata & access-list floors, EIP-7708 transfer
//! logs) introduced a large surface area where the two paths diverge in
//! plumbing even though they share the same VM core. Notable risk areas:
//!
//! - Mempool admission gas checks that must match VM intrinsic charges exactly,
//!   so the builder never admits a tx the VM would later reject, and never
//!   rejects a tx the VM would accept (EIP-8037 CREATE intrinsic split).
//! - BAL recording sites vs. BAL validation sites — the builder records via
//!   `bal_recorder` callsites (gated on post-gas-check conditions); the
//!   validator runs a shadow recorder on per-tx `tx_db` and diffs against the
//!   header BAL.
//! - The 2D inclusion check (EIP-8037 PR #2703) must fire at the same running
//!   totals on the builder (`fill_transactions`) and the validator
//!   (`execute_block_parallel` aggregation loop).
//! - Net-zero balance / storage filtering, coinbase handling when priority fee
//!   is zero, SYSTEM_ADDRESS filtering for pre-exec system calls vs. user-tx
//!   accesses.
//! - State-gas reservoir semantics across revert / success: the builder
//!   maintains a `bal_checkpoint` across rejected txs; the validator maintains
//!   an equivalent snapshot per frame.
//!
//! # How a test fails
//!
//! Each test seeds an Amsterdam-at-genesis chain, puts one or more txs into the
//! mempool, drives the payload builder to produce a block, and then hands the
//! result (block + BAL) back to the validator pipeline via
//! `add_block_pipeline_bal`. A failure therefore surfaces as one of:
//!
//! - `build_payload` panic / error — the builder could not even produce a
//!   block from the mempool contents (possible regression in the builder).
//! - Built-BAL-hash vs. header-BAL-hash mismatch (the builder is inconsistent
//!   with itself, almost certainly a bug in the BAL finalization step).
//! - Validator rejection (`add_block_pipeline_bal` returns Err) — the parity
//!   is broken. The error message identifies which check fired.
//!
//! When a test in this module breaks, treat it as a P0 before merging: a green
//! ef-tests blockchain suite does not catch builder/validator drift because
//! ef-tests only consume blocks, never produce them.
//!
//! # Scenario coverage
//!
//! The module has two groups of tests.
//!
//! **Positive parity** — builder produces a legitimate block, validator must
//! accept. Guards against silent drift (e.g., someone changes a recording
//! site in the builder but not the check in the validator, or changes the
//! intrinsic gas formula in one path but not the other):
//!
//! - `parity_empty_block` — pre-exec system calls; SYSTEM_ADDRESS filter.
//! - `parity_simple_transfer` — smoke; balance changes, coinbase handling.
//! - `parity_create_tx` — Amsterdam CREATE intrinsic split (EIP-8037 PR #2687).
//! - `parity_sstore_zero_to_nonzero` — state gas for fresh storage (EIP-8037).
//! - `parity_balance_of_unused_account` — pure-access BAL entry (EIP-7928).
//! - `parity_large_calldata_floor` — EIP-7976 calldata floor (16 gas/byte).
//! - `parity_access_list_floor` — EIP-7981 access-list data fold-in.
//! - `parity_user_tx_touches_system_address` — SYSTEM_ADDRESS in BAL when
//!   legitimately touched by user code via `EXTCODEHASH`.
//! - `parity_multiple_txs_different_senders` — BAL aggregation across txs,
//!   net-zero filter flush on tx boundary.
//!
//! **Negative parity** — builder produces a legitimate block, we CORRUPT the
//! BAL (remove an entry / append a surplus entry) and re-hash the header,
//! then hand it to the validator. The validator must reject. Each scenario
//! mirrors one of the Hive `test_bal_invalid_*` cases we fixed in session 3;
//! if any of these flips to "accept", the corresponding BAL validation check
//! has regressed:
//!
//! - `parity_reject_missing_pure_access_account` → Hive
//!   `test_bal_invalid_missing_account[access_only]`. Validator must reject
//!   when a user-tx `BALANCE`-probed address is missing from the BAL.
//! - `parity_reject_surplus_system_address` → Hive
//!   `test_bal_invalid_surplus_system_address_from_system_call`. Validator
//!   must reject when the BAL contains `SYSTEM_ADDRESS` without any user tx
//!   touching it (system-call-only).
//! - `parity_reject_missing_storage_read` → Hive
//!   `test_bal_invalid_field_entries[missing_storage_read]`. Validator must
//!   reject when a `SLOAD`-ed slot is missing from `storage_reads`.
//! - `parity_reject_missing_storage_change` → Hive
//!   `test_bal_invalid_field_entries[missing_storage_change]`. Validator must
//!   reject when an `SSTORE`-written slot is missing from `storage_changes`.
//! - `parity_reject_missing_code_change` → Hive
//!   `test_bal_invalid_field_entries[missing_code_change]`. Validator must
//!   reject when a `CREATE`d contract's `code_changes` entry is missing
//!   (guards the PR-#6463-adjacent PART B pre-state fallback added in
//!   session 3).
//!
//! Future additions should continue to target specific spec mechanisms rather
//! than broad coverage: every scenario we add costs CI time, so each test
//! should guard against at least one concrete spec rule or one known drift
//! risk. See also `TODO.md` for the remaining test gaps documented by the
//! session-3 reviewer agents.

use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, PayloadBuildResult, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    constants::SYSTEM_ADDRESS,
    types::{
        AccessList, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction,
        ELASTICITY_MULTIPLIER, Genesis, GenesisAccount, Transaction, TxKind,
    },
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

/// Test private key from fixtures/keys/private_keys_tests.txt.
const TEST_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
const TEST_MAX_FEE_PER_GAS: u64 = 10_000_000_000;
const TEST_GAS_LIMIT: u64 = 200_000;
/// Timestamp offset between parent (genesis=0) and the built block.
const TEST_BLOCK_TIMESTAMP: u64 = 12;

fn test_secret_key() -> SecretKey {
    SecretKey::from_slice(&hex::decode(TEST_PRIVATE_KEY).unwrap()).unwrap()
}

fn sender_from_key(sk: &SecretKey) -> Address {
    LocalSigner::new(*sk).address
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Loads the execution-api genesis, forces Amsterdam activation at genesis
/// (patching all intermediate fork times to 0), seeds `sender` with funds,
/// and optionally inserts additional accounts.
async fn setup_amsterdam_store(
    sender: Address,
    extra_accounts: &[(Address, GenesisAccount)],
) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("genesis file");
    let mut genesis: Genesis = serde_json::from_reader(BufReader::new(file)).expect("genesis json");

    // Ensure every fork up to and including Amsterdam is active at timestamp 0.
    genesis.config.shanghai_time = Some(0);
    genesis.config.cancun_time = Some(0);
    genesis.config.prague_time = Some(0);
    genesis.config.osaka_time = Some(0);
    genesis.config.bpo1_time = Some(0);
    genesis.config.bpo2_time = Some(0);
    genesis.config.amsterdam_time = Some(0);

    let chain_id = genesis.config.chain_id;

    genesis.alloc.insert(
        sender,
        GenesisAccount {
            balance: U256::from(10).pow(U256::from(20)), // 100 ETH
            code: Bytes::new(),
            storage: Default::default(),
            nonce: 0,
        },
    );
    for (addr, acc) in extra_accounts {
        genesis.alloc.insert(*addr, acc.clone());
    }

    let mut store = Store::new("store.db", EngineType::InMemory).expect("in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("seed genesis");
    (store, chain_id)
}

fn build_args(parent_header: &BlockHeader) -> BuildPayloadArgs {
    BuildPayloadArgs {
        parent: parent_header.hash(),
        timestamp: parent_header.timestamp + TEST_BLOCK_TIMESTAMP,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        inclusion_list_transactions: None,
    }
}

/// Builds a block via the payload builder, then runs it through the validator
/// pipeline on the same store with the built BAL as the header BAL. Returns
/// the build result for any extra per-test assertions.
fn build_and_validate(
    store: &Store,
    blockchain: &Blockchain,
    parent_header: &BlockHeader,
) -> PayloadBuildResult {
    let block =
        create_payload(&build_args(parent_header), store, Bytes::new()).expect("create_payload");
    let result = blockchain.build_payload(block).expect("build_payload");

    // Sanity: Amsterdam blocks must carry a BAL and the header hash must match.
    let bal = result
        .block_access_list
        .as_ref()
        .expect("Amsterdam block must have BAL");
    let header_hash = result
        .payload
        .header
        .block_access_list_hash
        .expect("Amsterdam block header must commit to a BAL hash");
    assert_eq!(
        header_hash,
        bal.compute_hash(),
        "header BAL hash must match the built BAL"
    );

    // Hand the built block + BAL to the validator pipeline. If the validator
    // rejects what the builder produced we'd miss a slot on devnet.
    let produced_bal = blockchain
        .add_block_pipeline_bal(result.payload.clone(), Some(bal))
        .expect("validator pipeline must accept a builder-produced block");

    // The validator doesn't rebuild the BAL when header_bal is Some — it
    // returns None for the produced BAL in that path. Tolerate both.
    if let Some(validator_bal) = produced_bal {
        assert_eq!(
            validator_bal.compute_hash(),
            bal.compute_hash(),
            "validator-produced BAL must match the builder's BAL"
        );
    }

    result
}

async fn amsterdam_genesis_header(store: &Store) -> BlockHeader {
    store
        .get_block_header(0)
        .unwrap()
        .expect("genesis header must exist")
}

/// Signs an EIP-1559 tx and puts it in the mempool.
async fn push_tx(
    blockchain: &Blockchain,
    signer: &Signer,
    tx: EIP1559Transaction,
) -> Result<H256, Box<dyn std::error::Error>> {
    let mut tx = Transaction::EIP1559Transaction(tx);
    tx.sign_inplace(signer).await?;
    Ok(blockchain.add_transaction_to_pool(tx).await?)
}

/// Builds a block via the payload builder without validating. Used by the
/// negative-parity tests that corrupt the BAL before feeding it back to the
/// validator.
fn build_only(
    store: &Store,
    blockchain: &Blockchain,
    parent_header: &BlockHeader,
) -> PayloadBuildResult {
    let block =
        create_payload(&build_args(parent_header), store, Bytes::new()).expect("create_payload");
    blockchain.build_payload(block).expect("build_payload")
}

/// Takes a legitimate `PayloadBuildResult`, applies a BAL-corrupting `mutator`
/// to the built BAL, re-hashes it into the header, and feeds the corrupted
/// block to the validator pipeline. Returns the validator error (expected).
fn validate_corrupted_bal(
    blockchain: &Blockchain,
    mut result: PayloadBuildResult,
    mutator: impl FnOnce(&mut ethrex_common::types::block_access_list::BlockAccessList),
) -> ethrex_blockchain::error::ChainError {
    let mut bal = result
        .block_access_list
        .take()
        .expect("Amsterdam build must produce BAL");
    mutator(&mut bal);
    // Rewrite the header hash so the corrupted BAL is the one the validator
    // compares against — otherwise the hash check rejects before the BAL
    // validation logic even runs, which is not what we're testing here.
    result.payload.header.block_access_list_hash = Some(bal.compute_hash());

    blockchain
        .add_block_pipeline_bal(result.payload, Some(&bal))
        .expect_err("validator must reject the corrupted BAL")
}

/// Removes the entire `AccountChanges` entry for `addr` from the BAL.
fn drop_account(bal: &mut ethrex_common::types::block_access_list::BlockAccessList, addr: Address) {
    let accounts: Vec<_> = bal
        .accounts()
        .iter()
        .filter(|a| a.address != addr)
        .cloned()
        .collect();
    *bal = ethrex_common::types::block_access_list::BlockAccessList::from_accounts(accounts);
}

/// Clears one of the sub-lists on the account entry matching `addr`. The
/// BlockAccessList is rebuilt from scratch so the canonical ordering /
/// checkpoint state stays consistent with its hash.
fn mutate_account(
    bal: &mut ethrex_common::types::block_access_list::BlockAccessList,
    addr: Address,
    mutator: impl FnOnce(&mut ethrex_common::types::block_access_list::AccountChanges),
) {
    let mut accounts: Vec<_> = bal.accounts().to_vec();
    let acct = accounts
        .iter_mut()
        .find(|a| a.address == addr)
        .expect("target account must exist in BAL");
    mutator(acct);
    *bal = ethrex_common::types::block_access_list::BlockAccessList::from_accounts(accounts);
}

/// Appends a brand-new bare account entry to the BAL. Used to simulate a
/// malicious / buggy builder that adds an address with no corresponding
/// execution access (e.g., the `surplus_system_address` case).
fn append_bare_account(
    bal: &mut ethrex_common::types::block_access_list::BlockAccessList,
    addr: Address,
) {
    use ethrex_common::types::block_access_list::AccountChanges;
    let mut accounts: Vec<_> = bal.accounts().to_vec();
    accounts.push(AccountChanges {
        address: addr,
        storage_changes: Vec::new(),
        storage_reads: Vec::new(),
        balance_changes: Vec::new(),
        nonce_changes: Vec::new(),
        code_changes: Vec::new(),
    });
    // Keep addresses sorted per EIP-7928 canonical form.
    accounts.sort_by_key(|a| a.address);
    *bal = ethrex_common::types::block_access_list::BlockAccessList::from_accounts(accounts);
}

// ---------------- Tests ----------------

/// An empty Amsterdam block (no user txs, only the pre-exec system calls that
/// populate beacon_root and block_hash_history). Verifies the builder/validator
/// agree on system-call BAL entries and SYSTEM_ADDRESS is correctly filtered.
#[tokio::test]
async fn parity_empty_block() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let (store, _chain_id) = setup_amsterdam_store(sender, &[]).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let parent = amsterdam_genesis_header(&store).await;

    let result = build_and_validate(&store, &blockchain, &parent);
    assert!(
        result.payload.body.transactions.is_empty(),
        "empty block must have no txs"
    );

    // EIP-7928: SYSTEM_ADDRESS must NOT appear in a valid BAL produced solely
    // from pre-exec system calls.
    let bal = result.block_access_list.as_ref().unwrap();
    assert!(
        !bal.accounts()
            .iter()
            .any(|acct| acct.address == SYSTEM_ADDRESS),
        "BAL must not contain SYSTEM_ADDRESS for system-call-only activity"
    );
}

/// A simple value transfer (no state creation, no refunds). Smoke test for the
/// common case and verifies recipient appears as a balance change.
#[tokio::test]
async fn parity_simple_transfer() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();
    let recipient = Address::from_low_u64_be(0xBEEF);

    let (store, chain_id) = setup_amsterdam_store(sender, &[]).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let parent = amsterdam_genesis_header(&store).await;

    push_tx(
        &blockchain,
        &signer,
        EIP1559Transaction {
            chain_id,
            nonce: 0,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
            gas_limit: TEST_GAS_LIMIT,
            to: TxKind::Call(recipient),
            value: U256::from(10u64.pow(15)),
            data: Bytes::new(),
            ..Default::default()
        },
    )
    .await
    .expect("tx pool");

    let result = build_and_validate(&store, &blockchain, &parent);
    assert_eq!(result.payload.body.transactions.len(), 1);
}

/// CREATE transaction. Exercises the Amsterdam intrinsic gas split
/// (REGULAR_GAS_CREATE + STATE_BYTES_PER_NEW_ACCOUNT * cpsb) on both the
/// builder (mempool admission + payload VM) and the validator.
#[tokio::test]
async fn parity_create_tx() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let (store, chain_id) = setup_amsterdam_store(sender, &[]).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let parent = amsterdam_genesis_header(&store).await;

    // Tiny runtime: PUSH1 0 PUSH1 0 RETURN → deploys zero bytes.
    // Init code: PUSH1 0x00 PUSH1 0x00 RETURN + pad.
    let init_code = Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xF3]);

    push_tx(
        &blockchain,
        &signer,
        EIP1559Transaction {
            chain_id,
            nonce: 0,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
            gas_limit: 500_000,
            to: TxKind::Create,
            value: U256::zero(),
            data: init_code,
            ..Default::default()
        },
    )
    .await
    .expect("tx pool");

    let result = build_and_validate(&store, &blockchain, &parent);
    assert_eq!(result.payload.body.transactions.len(), 1);
}

/// SSTORE 0 → 1 writes to a fresh slot in a pre-deployed contract.
/// Exercises state gas accounting (STATE_BYTES_PER_STORAGE_SET * cpsb) and
/// verifies builder/validator agree on storage_changes entries.
#[tokio::test]
async fn parity_sstore_zero_to_nonzero() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let target = Address::from_low_u64_be(0xC0DE);
    // PUSH1 0x01 PUSH1 0x00 SSTORE STOP
    let code = Bytes::from(vec![0x60, 0x01, 0x60, 0x00, 0x55, 0x00]);
    let (store, chain_id) = setup_amsterdam_store(
        sender,
        &[(
            target,
            GenesisAccount {
                balance: U256::zero(),
                code,
                storage: Default::default(),
                nonce: 1,
            },
        )],
    )
    .await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let parent = amsterdam_genesis_header(&store).await;

    push_tx(
        &blockchain,
        &signer,
        EIP1559Transaction {
            chain_id,
            nonce: 0,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
            gas_limit: TEST_GAS_LIMIT,
            to: TxKind::Call(target),
            value: U256::zero(),
            data: Bytes::new(),
            ..Default::default()
        },
    )
    .await
    .expect("tx pool");

    let result = build_and_validate(&store, &blockchain, &parent);
    assert_eq!(result.payload.body.transactions.len(), 1);
}

/// Contract reads the balance of an otherwise-untouched account. The target
/// address must appear in the BAL as a pure-access entry (no changes). The
/// shadow recorder in the validator must match the builder's decision.
#[tokio::test]
async fn parity_balance_of_unused_account() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let probed = Address::from_low_u64_be(0xCAFE);
    let checker = Address::from_low_u64_be(0xC0DE);

    // PUSH20 <probed> BALANCE POP STOP
    let mut code = Vec::with_capacity(24);
    code.push(0x73); // PUSH20
    code.extend_from_slice(probed.as_bytes());
    code.push(0x31); // BALANCE
    code.push(0x50); // POP
    code.push(0x00); // STOP

    let (store, chain_id) = setup_amsterdam_store(
        sender,
        &[
            (
                checker,
                GenesisAccount {
                    balance: U256::zero(),
                    code: Bytes::from(code),
                    storage: Default::default(),
                    nonce: 1,
                },
            ),
            (
                probed,
                GenesisAccount {
                    balance: U256::from(7),
                    code: Bytes::new(),
                    storage: Default::default(),
                    nonce: 0,
                },
            ),
        ],
    )
    .await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let parent = amsterdam_genesis_header(&store).await;

    push_tx(
        &blockchain,
        &signer,
        EIP1559Transaction {
            chain_id,
            nonce: 0,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
            gas_limit: TEST_GAS_LIMIT,
            to: TxKind::Call(checker),
            value: U256::zero(),
            data: Bytes::new(),
            ..Default::default()
        },
    )
    .await
    .expect("tx pool");

    let result = build_and_validate(&store, &blockchain, &parent);
    let bal = result.block_access_list.as_ref().unwrap();
    assert!(
        bal.accounts().iter().any(|acct| acct.address == probed),
        "BALANCE target must appear in BAL as pure-access entry"
    );
}

/// Calldata-heavy transaction exercising the EIP-7976 (64-gas-per-byte) floor.
/// Builder mempool admission and VM charge must agree on the same intrinsic
/// gas; the builder/validator must both account the same regular-dim block
/// gas (`max(tx_regular, calldata_floor)`).
#[tokio::test]
async fn parity_large_calldata_floor() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let (store, chain_id) = setup_amsterdam_store(sender, &[]).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let parent = amsterdam_genesis_header(&store).await;

    // 512 bytes of calldata. Floor = 512 * 16 = 8192 gas on top of base.
    let calldata = Bytes::from(vec![0x55u8; 512]);

    push_tx(
        &blockchain,
        &signer,
        EIP1559Transaction {
            chain_id,
            nonce: 0,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
            gas_limit: TEST_GAS_LIMIT,
            to: TxKind::Call(Address::from_low_u64_be(0xBEEF)),
            value: U256::zero(),
            data: calldata,
            ..Default::default()
        },
    )
    .await
    .expect("tx pool");

    let result = build_and_validate(&store, &blockchain, &parent);
    assert_eq!(result.payload.body.transactions.len(), 1);
}

/// EIP-7981: access-list data bytes fold into the floor-token count. Builder
/// mempool admission and VM charge must both account the access-list data at
/// 64 gas/byte, and the validator must accept the resulting block.
#[tokio::test]
async fn parity_access_list_floor() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let (store, chain_id) = setup_amsterdam_store(sender, &[]).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let parent = amsterdam_genesis_header(&store).await;

    let access_list: AccessList = vec![
        (
            Address::from_low_u64_be(0x11),
            vec![H256::from_low_u64_be(1), H256::from_low_u64_be(2)],
        ),
        (
            Address::from_low_u64_be(0x22),
            vec![H256::from_low_u64_be(3)],
        ),
    ];

    push_tx(
        &blockchain,
        &signer,
        EIP1559Transaction {
            chain_id,
            nonce: 0,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
            gas_limit: TEST_GAS_LIMIT,
            to: TxKind::Call(Address::from_low_u64_be(0xBEEF)),
            value: U256::zero(),
            data: Bytes::new(),
            access_list,
            ..Default::default()
        },
    )
    .await
    .expect("tx pool");

    let result = build_and_validate(&store, &blockchain, &parent);
    assert_eq!(result.payload.body.transactions.len(), 1);
}

/// User tx that touches SYSTEM_ADDRESS via EXTCODEHASH. SYSTEM_ADDRESS MUST
/// appear in the BAL (user-tx access legitimizes it), and validator must
/// agree.
#[tokio::test]
async fn parity_user_tx_touches_system_address() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let toucher = Address::from_low_u64_be(0xC0DE);
    // PUSH20 <SYSTEM_ADDRESS> EXTCODEHASH POP STOP
    let mut code = Vec::with_capacity(24);
    code.push(0x73); // PUSH20
    code.extend_from_slice(SYSTEM_ADDRESS.as_bytes());
    code.push(0x3F); // EXTCODEHASH
    code.push(0x50); // POP
    code.push(0x00); // STOP

    let (store, chain_id) = setup_amsterdam_store(
        sender,
        &[(
            toucher,
            GenesisAccount {
                balance: U256::zero(),
                code: Bytes::from(code),
                storage: Default::default(),
                nonce: 1,
            },
        )],
    )
    .await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let parent = amsterdam_genesis_header(&store).await;

    push_tx(
        &blockchain,
        &signer,
        EIP1559Transaction {
            chain_id,
            nonce: 0,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
            gas_limit: TEST_GAS_LIMIT,
            to: TxKind::Call(toucher),
            value: U256::zero(),
            data: Bytes::new(),
            ..Default::default()
        },
    )
    .await
    .expect("tx pool");

    let result = build_and_validate(&store, &blockchain, &parent);
    let bal = result.block_access_list.as_ref().unwrap();
    assert!(
        bal.accounts()
            .iter()
            .any(|acct| acct.address == SYSTEM_ADDRESS),
        "user-tx touch of SYSTEM_ADDRESS must land in BAL"
    );
}

/// Multiple independent txs from different senders. Confirms builder and
/// validator agree on BAL aggregation across txs (cumulative addr_to_idx,
/// per-tx bal_index assignment, net-zero filter flush between txs).
#[tokio::test]
async fn parity_multiple_txs_different_senders() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let sk2 = SecretKey::from_slice(
        &hex::decode("11234567812345678123456781234567812345678123456781234567812345aa").unwrap(),
    )
    .unwrap();
    let sender2 = sender_from_key(&sk2);
    let signer2: Signer = LocalSigner::new(sk2).into();

    let (store, chain_id) = setup_amsterdam_store(
        sender,
        &[(
            sender2,
            GenesisAccount {
                balance: U256::from(10).pow(U256::from(20)),
                code: Bytes::new(),
                storage: Default::default(),
                nonce: 0,
            },
        )],
    )
    .await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let parent = amsterdam_genesis_header(&store).await;

    let dest = Address::from_low_u64_be(0xBEEF);
    for (i, signer_ref) in [&signer, &signer2].into_iter().enumerate() {
        push_tx(
            &blockchain,
            signer_ref,
            EIP1559Transaction {
                chain_id,
                nonce: 0,
                max_priority_fee_per_gas: 1,
                max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
                gas_limit: 21_000,
                to: TxKind::Call(dest),
                value: U256::from((i as u64 + 1) * 100),
                data: Bytes::new(),
                ..Default::default()
            },
        )
        .await
        .expect("tx pool");
    }

    let result = build_and_validate(&store, &blockchain, &parent);
    assert_eq!(
        result.payload.body.transactions.len(),
        2,
        "both txs must be included"
    );
}

// ---------------- Negative parity tests ----------------
//
// Each test below builds a legitimate Amsterdam block, then corrupts the BAL
// (remove an entry / add a surplus entry) in a way that mirrors one of the
// Hive `test_bal_invalid_*` scenarios we fixed this session. The validator
// pipeline must reject the corrupted block. If one of these flips to "accept",
// the corresponding BAL validation check has regressed.

/// Hive parity: `test_bal_invalid_missing_account[access_only]`.
/// User tx reads `BALANCE(probed)`; BAL must contain `probed`. Remove it from
/// the BAL and expect the shadow-recorder missing-access check to fire.
#[tokio::test]
async fn parity_reject_missing_pure_access_account() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let probed = Address::from_low_u64_be(0xCAFE);
    let checker = Address::from_low_u64_be(0xC0DE);
    let mut code = Vec::with_capacity(24);
    code.push(0x73); // PUSH20
    code.extend_from_slice(probed.as_bytes());
    code.push(0x31); // BALANCE
    code.push(0x50); // POP
    code.push(0x00); // STOP

    let (store, chain_id) = setup_amsterdam_store(
        sender,
        &[
            (
                checker,
                GenesisAccount {
                    balance: U256::zero(),
                    code: Bytes::from(code),
                    storage: Default::default(),
                    nonce: 1,
                },
            ),
            (
                probed,
                GenesisAccount {
                    balance: U256::from(7),
                    code: Bytes::new(),
                    storage: Default::default(),
                    nonce: 0,
                },
            ),
        ],
    )
    .await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let parent = amsterdam_genesis_header(&store).await;

    push_tx(
        &blockchain,
        &signer,
        EIP1559Transaction {
            chain_id,
            nonce: 0,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
            gas_limit: TEST_GAS_LIMIT,
            to: TxKind::Call(checker),
            value: U256::zero(),
            data: Bytes::new(),
            ..Default::default()
        },
    )
    .await
    .expect("tx pool");

    let result = build_only(&store, &blockchain, &parent);
    let err = validate_corrupted_bal(&blockchain, result, |bal| drop_account(bal, probed));
    let msg = format!("{err}");
    assert!(
        msg.contains("BAL validation failed") && msg.contains("missing from BAL"),
        "expected missing-access rejection, got: {msg}"
    );
}

/// Hive parity: `test_bal_invalid_surplus_system_address_from_system_call`.
/// Empty Amsterdam block; corrupt the BAL by appending a bare SYSTEM_ADDRESS
/// entry. Extraneous-entry logic must reject it (SYSTEM_ADDRESS is no longer
/// whitelisted from the `unaccessed_pure_accounts` checks).
#[tokio::test]
async fn parity_reject_surplus_system_address() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let (store, _chain_id) = setup_amsterdam_store(sender, &[]).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let parent = amsterdam_genesis_header(&store).await;

    let result = build_only(&store, &blockchain, &parent);
    let err = validate_corrupted_bal(&blockchain, result, |bal| {
        append_bare_account(bal, SYSTEM_ADDRESS)
    });
    let msg = format!("{err}");
    assert!(
        msg.contains("BAL validation failed"),
        "expected BAL extraneous-entry rejection, got: {msg}"
    );
}

/// Hive parity: `test_bal_invalid_field_entries[missing_storage_read]`.
/// Tx does `SLOAD(slot)` on an oracle contract; BAL must carry the slot in
/// `storage_reads`. Remove the entry and expect rejection by the shadow-
/// recorder storage_reads check.
#[tokio::test]
async fn parity_reject_missing_storage_read() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let oracle = Address::from_low_u64_be(0xC0DE);
    // Contract: PUSH1 0x02 SLOAD POP STOP (reads slot 2).
    let code = Bytes::from(vec![0x60, 0x02, 0x54, 0x50, 0x00]);
    let mut storage = std::collections::BTreeMap::new();
    storage.insert(U256::from(2), U256::from(0x84));

    let (store, chain_id) = setup_amsterdam_store(
        sender,
        &[(
            oracle,
            GenesisAccount {
                balance: U256::zero(),
                code,
                storage,
                nonce: 1,
            },
        )],
    )
    .await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let parent = amsterdam_genesis_header(&store).await;

    push_tx(
        &blockchain,
        &signer,
        EIP1559Transaction {
            chain_id,
            nonce: 0,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
            gas_limit: TEST_GAS_LIMIT,
            to: TxKind::Call(oracle),
            value: U256::zero(),
            data: Bytes::new(),
            ..Default::default()
        },
    )
    .await
    .expect("tx pool");

    let result = build_only(&store, &blockchain, &parent);
    let err = validate_corrupted_bal(&blockchain, result, |bal| {
        mutate_account(bal, oracle, |acct| {
            acct.storage_reads.clear();
        })
    });
    let msg = format!("{err}");
    assert!(
        msg.contains("BAL validation failed")
            && (msg.contains("was read during execution") || msg.contains("storage_reads")),
        "expected missing storage-read rejection, got: {msg}"
    );
}

/// Hive parity: `test_bal_invalid_field_entries[missing_storage_change]`.
/// Tx writes a storage slot; BAL must carry the slot in `storage_changes`.
/// Remove the entry and expect rejection.
#[tokio::test]
async fn parity_reject_missing_storage_change() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let target = Address::from_low_u64_be(0xC0DE);
    // PUSH1 0x01 PUSH1 0x00 SSTORE STOP
    let code = Bytes::from(vec![0x60, 0x01, 0x60, 0x00, 0x55, 0x00]);
    let (store, chain_id) = setup_amsterdam_store(
        sender,
        &[(
            target,
            GenesisAccount {
                balance: U256::zero(),
                code,
                storage: Default::default(),
                nonce: 1,
            },
        )],
    )
    .await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let parent = amsterdam_genesis_header(&store).await;

    push_tx(
        &blockchain,
        &signer,
        EIP1559Transaction {
            chain_id,
            nonce: 0,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
            gas_limit: TEST_GAS_LIMIT,
            to: TxKind::Call(target),
            value: U256::zero(),
            data: Bytes::new(),
            ..Default::default()
        },
    )
    .await
    .expect("tx pool");

    let result = build_only(&store, &blockchain, &parent);
    let err = validate_corrupted_bal(&blockchain, result, |bal| {
        mutate_account(bal, target, |acct| {
            acct.storage_changes.clear();
        })
    });
    let msg = format!("{err}");
    assert!(
        msg.contains("BAL validation failed"),
        "expected missing storage-change rejection, got: {msg}"
    );
}

/// Hive parity: `test_bal_invalid_field_entries[missing_code_change]`.
/// CREATE tx deploys a contract; BAL must carry a `code_changes` entry for
/// the created address. Clear that entry and expect rejection from the
/// pre-state fallback added in `validate_tx_execution` PART B.
#[tokio::test]
async fn parity_reject_missing_code_change() {
    use ethrex_common::evm::calculate_create_address;
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let (store, chain_id) = setup_amsterdam_store(sender, &[]).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let parent = amsterdam_genesis_header(&store).await;

    // Init code that deploys 1 byte (0x00 = STOP). Produces a non-empty
    // code_hash, so clearing `code_changes` in the BAL causes the PART B
    // code check to compare against the pre-state EMPTY_KECCAK_HASH and
    // reject (matching EELS behavior).
    //
    //   PUSH1 0x00 (value to store)
    //   PUSH1 0x00 (memory offset)
    //   MSTORE8    (store 1 byte at offset 0)
    //   PUSH1 0x01 (size)
    //   PUSH1 0x00 (offset)
    //   RETURN     (return memory[0..1] as the deployed code)
    let init_code = Bytes::from(vec![
        0x60, 0x00, 0x60, 0x00, 0x53, 0x60, 0x01, 0x60, 0x00, 0xF3,
    ]);
    let created = calculate_create_address(sender, 0);

    push_tx(
        &blockchain,
        &signer,
        EIP1559Transaction {
            chain_id,
            nonce: 0,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
            gas_limit: 500_000,
            to: TxKind::Create,
            value: U256::zero(),
            data: init_code,
            ..Default::default()
        },
    )
    .await
    .expect("tx pool");

    let result = build_only(&store, &blockchain, &parent);
    let err = validate_corrupted_bal(&blockchain, result, |bal| {
        mutate_account(bal, created, |acct| {
            acct.code_changes.clear();
        })
    });
    let msg = format!("{err}");
    assert!(
        msg.contains("BAL validation failed"),
        "expected missing code-change rejection, got: {msg}"
    );
}
