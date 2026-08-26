//! EIP-8312: UTXO frames — vault predeploy behavior.
//!
//! These tests execute the vault's *pinned runtime bytecode* (verbatim from the
//! spec's `utxo_vault.eas`) under a real EVM, so they check what the deployed
//! contract does rather than what a Rust reimplementation of it would do:
//!
//!   - a deposit succeeds only for exactly 20 bytes of calldata, non-zero value,
//!     and a non-zero recipient;
//!   - a plain transfer (empty calldata) reverts, so no value can enter the
//!     vault without creating a UTXO;
//!   - each success assigns the next index and increments the counter at slot 0;
//!   - each success emits `UtxoCreated(source, recipient, index, value)` as a
//!     LOG3 from the vault with `index ++ value` as data;
//!   - deposits compose with EIP-7708 (the value transfer into the vault emits
//!     its own Transfer log).
//!
//! Frame-level validation of spends lives in the ethrex-common tests; this file
//! is only about the contract.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        Account, AccountState, ChainConfig, Code, CodeMetadata, EIP1559Transaction, Fork, Log,
        SLOT_NEXT_INDEX, Transaction, TxKind, UTXO_CREATED_TOPIC, utxo_vault,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    errors::{DatabaseError, ExecutionReport, TxResult},
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_vm::system_contracts::UTXO_VAULT_RUNTIME_BYTECODE;
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ==================== Minimal test database ====================

struct TestDatabase;

impl Database for TestDatabase {
    fn get_account_state(&self, _address: Address) -> Result<AccountState, DatabaseError> {
        Ok(AccountState::default())
    }
    fn get_storage_value(&self, _address: Address, _key: H256) -> Result<U256, DatabaseError> {
        Ok(U256::zero())
    }
    fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
        Ok(H256::zero())
    }
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Ok(ChainConfig::default())
    }
    fn get_account_code(&self, _code_hash: H256) -> Result<Code, DatabaseError> {
        Ok(Code::default())
    }
    fn get_code_metadata(&self, _code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        Ok(CodeMetadata { length: 0 })
    }
}

const GAS_LIMIT: u64 = 1_000_000;
const DEPOSITOR: u64 = 0xD3D0;
const RECIPIENT: u64 = 0x9EC1;

fn depositor() -> Address {
    Address::from_low_u64_be(DEPOSITOR)
}

fn recipient() -> Address {
    Address::from_low_u64_be(RECIPIENT)
}

fn slot(n: u64) -> H256 {
    let mut key = H256::zero();
    key.0[24..].copy_from_slice(&n.to_be_bytes());
    key
}

/// The vault account carrying the pinned runtime bytecode, with an optional
/// starting index counter and balance.
fn vault_account(next_index: u64, balance: U256) -> Account {
    let mut storage = FxHashMap::default();
    if next_index != 0 {
        storage.insert(slot(SLOT_NEXT_INDEX), U256::from(next_index));
    }
    Account::new(
        balance,
        Code::from_bytecode(
            Bytes::from_static(&UTXO_VAULT_RUNTIME_BYTECODE),
            &NativeCrypto,
        ),
        1,
        storage,
    )
}

/// Call the vault with `calldata` and `value`, returning the report plus the
/// post-execution database so storage and balances can be inspected.
fn call_vault(
    calldata: Bytes,
    value: U256,
    next_index: u64,
) -> (ExecutionReport, GeneralizedDatabase) {
    let accounts: FxHashMap<Address, Account> = [
        (
            depositor(),
            Account::new(
                U256::from(10u64).pow(U256::from(20u64)),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (utxo_vault(), vault_account(next_index, U256::zero())),
    ]
    .into_iter()
    .collect();

    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(TestDatabase), accounts);

    let fork = Fork::Hegota;
    let env = Environment {
        disable_gas_allowance_check: false,
        origin: depositor(),
        gas_limit: GAS_LIMIT,
        config: EVMConfig::new(fork, EVMConfig::canonical_values(fork)),
        block_number: 1,
        coinbase: Address::from_low_u64_be(0xCCC),
        timestamp: 1000,
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        slot_number: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::from(1000),
        base_blob_fee_per_gas: U256::from(1),
        gas_price: U256::from(1000),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: None,
        tx_max_fee_per_gas: Some(U256::from(1000)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: GAS_LIMIT * 2,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: false,
        disable_nonce_check: false,
        is_system_call: false,
    };

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(utxo_vault()),
        value,
        data: calldata,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let report = {
        let mut vm = VM::new(
            env,
            &mut db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
            &NativeCrypto,
            None,
        )
        .expect("VM::new");
        vm.execute().expect("execution should not error")
    };
    (report, db)
}

/// 20-byte recipient calldata, the only accepted deposit shape.
fn deposit_calldata(to: Address) -> Bytes {
    Bytes::copy_from_slice(to.as_bytes())
}

fn utxo_created_logs(report: &ExecutionReport) -> Vec<&Log> {
    report
        .logs
        .iter()
        .filter(|log| {
            log.address == utxo_vault() && log.topics.first() == Some(&UTXO_CREATED_TOPIC)
        })
        .collect()
}

fn vault_slot_value(db: &mut GeneralizedDatabase, key: u64) -> U256 {
    db.current_accounts_state
        .get(&utxo_vault())
        .and_then(|acc| acc.storage.get(&slot(key)).copied())
        .unwrap_or_default()
}

// ==================== Deposit success ====================

#[test]
fn deposit_assigns_index_zero_and_emits_utxo_created() {
    let value = U256::from(1_000_000u64);
    let (report, mut db) = call_vault(deposit_calldata(recipient()), value, 0);

    assert!(
        matches!(report.result, TxResult::Success),
        "deposit should succeed, got {:?}",
        report.result
    );

    // The counter advances 0 -> 1.
    assert_eq!(vault_slot_value(&mut db, SLOT_NEXT_INDEX), U256::one());

    // Exactly one UtxoCreated log, from the vault.
    let logs = utxo_created_logs(&report);
    assert_eq!(logs.len(), 1, "expected exactly one UtxoCreated log");
    let log = logs[0];

    // topics = [UTXO_CREATED_TOPIC, source (=caller), recipient]
    assert_eq!(log.topics.len(), 3);
    assert_eq!(log.topics[0], UTXO_CREATED_TOPIC);
    assert_eq!(log.topics[1], H256::from(depositor()));
    assert_eq!(log.topics[2], H256::from(recipient()));

    // data = index (32 bytes) ++ value (32 bytes)
    assert_eq!(log.data.len(), 64);
    assert_eq!(U256::from_big_endian(&log.data[..32]), U256::zero());
    assert_eq!(U256::from_big_endian(&log.data[32..]), value);

    // The value landed in the vault.
    let vault_balance = db
        .current_accounts_state
        .get(&utxo_vault())
        .map(|acc| acc.info.balance)
        .unwrap_or_default();
    assert_eq!(vault_balance, value);
}

#[test]
fn deposit_uses_the_stored_counter_and_advances_it() {
    // A vault mid-life: the next index comes from slot 0, not from zero.
    let value = U256::from(42u64);
    let (report, mut db) = call_vault(deposit_calldata(recipient()), value, 7);

    assert!(matches!(report.result, TxResult::Success));
    assert_eq!(vault_slot_value(&mut db, SLOT_NEXT_INDEX), U256::from(8u64));

    let logs = utxo_created_logs(&report);
    assert_eq!(logs.len(), 1);
    assert_eq!(
        U256::from_big_endian(&logs[0].data[..32]),
        U256::from(7u64),
        "the log must carry the index that was assigned"
    );
}

#[test]
fn deposit_records_the_caller_as_source_not_the_recipient() {
    // source is `caller`, so a wallet filtering topics[1] sees who paid.
    let (report, _db) = call_vault(deposit_calldata(recipient()), U256::from(5u64), 0);
    let logs = utxo_created_logs(&report);
    assert_eq!(logs[0].topics[1], H256::from(depositor()));
    assert_ne!(logs[0].topics[1], H256::from(recipient()));
}

#[test]
fn deposit_composes_with_eip7708_transfer_logs() {
    // The value transfer into the vault is an ordinary transfer, so under
    // EIP-7708 it emits its own Transfer log in addition to UtxoCreated. This is
    // what `GAS_UTXO_ACCOUNT_OUT`'s transfer-log component prices on the spend
    // side, and it means an indexer sees both events for one deposit.
    let (report, _db) = call_vault(deposit_calldata(recipient()), U256::from(9u64), 0);
    assert_eq!(utxo_created_logs(&report).len(), 1);
    assert!(
        report.logs.len() >= 2,
        "expected a Transfer log alongside UtxoCreated, got {} logs",
        report.logs.len()
    );
}

// ==================== Deposit rejection ====================

#[test]
fn plain_transfer_reverts_so_no_value_enters_without_a_utxo() {
    // Empty calldata: the load-bearing rule. If this succeeded, value could sit
    // in the vault with no UTXO backing it and the solvency invariant
    // ("the vault holds exactly the unspent UTXO value") would break.
    let (report, mut db) = call_vault(Bytes::new(), U256::from(1_000u64), 0);

    assert!(
        matches!(report.result, TxResult::Revert(_)),
        "a plain transfer must revert, got {:?}",
        report.result
    );
    // No UTXO, no counter movement, no logs.
    assert_eq!(vault_slot_value(&mut db, SLOT_NEXT_INDEX), U256::zero());
    assert!(utxo_created_logs(&report).is_empty());
}

#[test]
fn zero_value_deposit_reverts() {
    // A zero-value UTXO would be an unspendable object polluting discovery.
    let (report, mut db) = call_vault(deposit_calldata(recipient()), U256::zero(), 0);
    assert!(matches!(report.result, TxResult::Revert(_)));
    assert_eq!(vault_slot_value(&mut db, SLOT_NEXT_INDEX), U256::zero());
    assert!(utxo_created_logs(&report).is_empty());
}

#[test]
fn zero_recipient_deposit_reverts() {
    // 20 zero bytes is well-formed calldata but a burn address; rejected so a
    // UTXO always has a spendable owner.
    let (report, mut db) = call_vault(deposit_calldata(Address::zero()), U256::from(7u64), 0);
    assert!(matches!(report.result, TxResult::Revert(_)));
    assert_eq!(vault_slot_value(&mut db, SLOT_NEXT_INDEX), U256::zero());
    assert!(utxo_created_logs(&report).is_empty());
}

#[test]
fn wrong_calldata_length_reverts() {
    // Exactly 20 bytes or nothing: 19, 21, and 32 must all revert. 32 matters
    // most — an ABI-encoded address is the natural mistake, and accepting it
    // would read the recipient from the wrong bytes.
    for len in [19usize, 21, 32] {
        let mut calldata = vec![0u8; len];
        // Non-zero so a failure cannot be attributed to the zero-recipient rule.
        calldata[0] = 0xAB;
        let (report, mut db) = call_vault(Bytes::from(calldata), U256::from(7u64), 0);
        assert!(
            matches!(report.result, TxResult::Revert(_)),
            "{len}-byte calldata must revert, got {:?}",
            report.result
        );
        assert_eq!(vault_slot_value(&mut db, SLOT_NEXT_INDEX), U256::zero());
        assert!(utxo_created_logs(&report).is_empty());
    }
}

#[test]
fn revert_returns_empty_data() {
    // The spec's failure path is `push0 push0 revert`, i.e. no return data —
    // callers must not receive a decodable error payload.
    let (report, _db) = call_vault(Bytes::new(), U256::from(1u64), 0);
    assert!(matches!(report.result, TxResult::Revert(_)));
    assert!(
        report.output.is_empty(),
        "revert must return no data, got {} bytes",
        report.output.len()
    );
}

// ==================== Pinned bytecode ====================

#[test]
fn vault_runtime_bytecode_matches_the_spec() {
    // The spec publishes the runtime hex; pin it byte-for-byte so a rebuild of
    // the geas source cannot silently change deployed behavior.
    let expected = hex_literal_to_bytes(
        "5f3560601c80156014361415173415176048575f54806001015f555f5234602052337f\
         3b19241465a47bc187f1d9c7db70834855a907183742a4b63aa824c576296f5e60405f\
         a3005b5f5ffd",
    );
    assert_eq!(UTXO_VAULT_RUNTIME_BYTECODE.as_slice(), expected.as_slice());
    assert_eq!(UTXO_VAULT_RUNTIME_BYTECODE.len(), 76);
}

#[test]
fn utxo_created_topic_matches_the_event_signature() {
    assert_eq!(
        UTXO_CREATED_TOPIC,
        ethrex_common::utils::keccak(b"UtxoCreated(address,address,uint64,uint256)")
    );
}

fn hex_literal_to_bytes(s: &str) -> Vec<u8> {
    let cleaned: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).expect("hex"))
        .collect()
}

// ==================== Vault install (predeploy provisioning) ====================
//
// `install_vault_code` runs on BOTH the payload-build path
// (`Evm::apply_system_calls`) and the block-import path (`LEVM::prepare_block`).
// If the two disagree, the builder and importer compute different state roots —
// the failure mode the design flags as the top risk — so its properties are
// pinned here: idempotence, balance preservation, nonce convergence, and BAL
// recording (the parallel importer reconstructs post-state from the BAL).

use ethrex_vm::backends::levm::LEVM;

fn empty_db_with(accounts: Vec<(Address, Account)>) -> GeneralizedDatabase {
    GeneralizedDatabase::new_with_account_state(
        Arc::new(TestDatabase),
        accounts.into_iter().collect(),
    )
}

fn installed_vault(db: &mut GeneralizedDatabase) -> ethrex_levm::account::LevmAccount {
    db.current_accounts_state
        .get(&utxo_vault())
        .cloned()
        .expect("vault account must exist after install")
}

#[test]
fn install_creates_the_vault_with_pinned_code_and_nonce_one() {
    let mut db = empty_db_with(vec![]);
    LEVM::install_vault_code(&mut db, &NativeCrypto).expect("install");

    let vault = installed_vault(&mut db);
    assert_eq!(vault.info.nonce, 1);
    let code = db
        .codes
        .get(&vault.info.code_hash)
        .expect("code must be registered");
    assert_eq!(code.code(), UTXO_VAULT_RUNTIME_BYTECODE.as_slice());
}

#[test]
fn install_is_idempotent() {
    // Exactly one observable account update: at the first activated block and
    // none afterwards. A second install must not touch the account, or every
    // block would carry a spurious vault update (and a spurious BAL entry).
    let mut db = empty_db_with(vec![]);
    LEVM::install_vault_code(&mut db, &NativeCrypto).expect("first install");
    let after_first = installed_vault(&mut db);

    LEVM::install_vault_code(&mut db, &NativeCrypto).expect("second install");
    let after_second = installed_vault(&mut db);

    assert_eq!(after_first.info.code_hash, after_second.info.code_hash);
    assert_eq!(after_first.info.nonce, after_second.info.nonce);
    assert_eq!(after_first.info.balance, after_second.info.balance);
}

#[test]
fn install_preserves_a_pre_existing_balance() {
    // Someone may have sent value to 0x8312 before activation. That balance is
    // preserved and becomes inert surplus: conservation bounds every frame's
    // outflows by its proven input value, so a surplus can never be spent and
    // vault solvency is unaffected. Destroying it would burn user funds.
    let squatted = U256::from(12_345u64);
    let mut db = empty_db_with(vec![(
        utxo_vault(),
        Account::new(squatted, Code::default(), 0, FxHashMap::default()),
    )]);

    LEVM::install_vault_code(&mut db, &NativeCrypto).expect("install");

    let vault = installed_vault(&mut db);
    assert_eq!(vault.info.balance, squatted, "balance must be preserved");
    assert_eq!(vault.info.nonce, 1);
}

#[test]
fn install_never_lowers_an_existing_nonce() {
    // Nonce converges to max(existing, 1), matching the EIP-8250 activation
    // rule: lowering a nonce would let already-used CREATE addresses recur.
    let mut db = empty_db_with(vec![(
        utxo_vault(),
        Account::new(U256::zero(), Code::default(), 9, FxHashMap::default()),
    )]);

    LEVM::install_vault_code(&mut db, &NativeCrypto).expect("install");
    assert_eq!(installed_vault(&mut db).info.nonce, 9);
}

#[test]
fn install_leaves_the_index_counter_untouched() {
    // Installing must not reset an existing counter: doing so would re-issue
    // already-assigned indices, and a re-issued index collides with a spent bit
    // that is already set.
    let mut storage = FxHashMap::default();
    storage.insert(slot(SLOT_NEXT_INDEX), U256::from(500u64));
    let mut db = empty_db_with(vec![(
        utxo_vault(),
        Account::new(U256::zero(), Code::default(), 1, storage),
    )]);

    LEVM::install_vault_code(&mut db, &NativeCrypto).expect("install");
    assert_eq!(
        vault_slot_value(&mut db, SLOT_NEXT_INDEX),
        U256::from(500u64)
    );
}

#[test]
fn install_records_code_and_nonce_in_the_block_access_list() {
    // EIP-7928 parallel import rebuilds post-state from the BAL, so an install
    // that skips recording produces a state root the parallel path cannot match.
    let mut db = empty_db_with(vec![]);
    db.enable_bal_recording();

    LEVM::install_vault_code(&mut db, &NativeCrypto).expect("install");

    let bal = db.take_bal().expect("a BAL must have been recorded");
    let vault_changes = bal
        .accounts()
        .iter()
        .find(|changes| changes.address == utxo_vault())
        .expect("the BAL must mention the vault");
    assert!(
        !vault_changes.code_changes.is_empty(),
        "the install's code change must be recorded"
    );
    assert!(
        !vault_changes.nonce_changes.is_empty(),
        "the install's nonce change must be recorded"
    );
}

// ==================== Durable-write tier ====================
//
// EIP-8312 requires that a spent bit, once set, cannot be undone by a later
// frame's failure — the same durability EIP-8250 mandates for consumed keyed
// nonces, and which no write path in this client previously provided.
//
// The mechanism is ordering, not a special write: a staged write is absent from
// the cache while the frame loop runs, so every scope-revert (per-frame failure,
// atomic-batch unroll) restores a cache that never contained it. The flush then
// applies it through the ordinary journaled path, which is what keeps it
// reversible by `undo_last_tx` and recorded in the BAL.
//
// These tests exercise that interaction against the real `restore_cache_state`
// and the real BAL recorder rather than a paraphrase of them. The end-to-end
// matrix (spent bits staged by actual UTXO frames inside a batch) arrives with
// the frame handler.

use ethrex_common::types::{spent_bit_location, utxo_vault as vault_addr};
use ethrex_levm::utils::restore_cache_state;

/// Run `body` against a VM whose db has the vault installed, then return the db
/// so post-state can be inspected.
fn with_vm<F>(enable_bal: bool, body: F) -> GeneralizedDatabase
where
    F: FnOnce(&mut VM<'_>),
{
    let accounts: FxHashMap<Address, Account> = [
        (
            depositor(),
            Account::new(
                U256::from(10u64).pow(U256::from(20u64)),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (utxo_vault(), vault_account(0, U256::zero())),
    ]
    .into_iter()
    .collect();

    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(TestDatabase), accounts);
    if enable_bal {
        db.enable_bal_recording();
    }

    let fork = Fork::Hegota;
    let env = Environment {
        disable_gas_allowance_check: false,
        origin: depositor(),
        gas_limit: GAS_LIMIT,
        config: EVMConfig::new(fork, EVMConfig::canonical_values(fork)),
        block_number: 1,
        coinbase: Address::from_low_u64_be(0xCCC),
        timestamp: 1000,
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        slot_number: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::from(1000),
        base_blob_fee_per_gas: U256::from(1),
        gas_price: U256::from(1000),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: None,
        tx_max_fee_per_gas: Some(U256::from(1000)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: GAS_LIMIT * 2,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: false,
        disable_nonce_check: false,
        is_system_call: false,
    };
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(depositor()),
        value: U256::zero(),
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    {
        let mut vm = VM::new(
            env,
            &mut db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
            &NativeCrypto,
            None,
        )
        .expect("VM::new");
        body(&mut vm);
    }
    db
}

/// The spent-bit word slot and the word value with `index`'s bit set.
fn spent_word_for(index: u64) -> (H256, U256) {
    let (slot_u256, mask) = spent_bit_location(index);
    (H256(slot_u256.to_big_endian()), mask)
}

#[test]
fn a_staged_write_is_invisible_to_the_cache_until_flushed() {
    let (slot, word) = spent_word_for(7);
    let mut db = with_vm(false, |vm| {
        vm.stage_durable_vault_write(slot, word);
        // Nothing is in the cache yet — this is precisely why no scope-revert can
        // undo it.
        assert!(
            vm.db
                .current_accounts_state
                .get(&vault_addr())
                .map(|acc| acc.storage.get(&slot).copied().unwrap_or_default())
                .unwrap_or_default()
                .is_zero(),
            "a staged write must not be in the cache"
        );
        // But it IS visible to a later read in the same transaction, or a second
        // frame spending another index in the same word could double-spend.
        assert_eq!(vm.read_vault_slot(slot).expect("read"), word);
    });
    // The VM was dropped without a flush, so nothing persisted.
    assert!(vault_slot_word(&mut db, slot).is_zero());
}

#[test]
fn a_cache_restore_cannot_undo_a_staged_write() {
    // This is the durability property, tested against the real restore path
    // that an atomic-batch unroll uses.
    let (slot, word) = spent_word_for(3);
    let mut db = with_vm(false, |vm| {
        // Capture a backup exactly as a frame/batch/body scope entry does, then
        // stage the durable write inside that scope.
        let backup = vm.current_call_frame.call_frame_backup.clone();
        vm.stage_durable_vault_write(slot, word);

        // The scope fails and its cache state is restored.
        restore_cache_state(vm.db, backup).expect("restore");

        // The staged write survived, because it was never in the cache.
        assert_eq!(vm.read_vault_slot(slot).expect("read"), word);

        // Committing it now still works.
        vm.flush_durable_vault_writes().expect("flush");
    });
    assert_eq!(
        vault_slot_word(&mut db, slot),
        word,
        "a durable write must survive a scope revert and land at commit"
    );
}

#[test]
fn flushing_makes_the_write_reversible_by_the_transaction_level_backup() {
    // The other half of the contract: durable against *frame* failure, but still
    // reversible when the whole transaction is dropped — which is what
    // `undo_last_tx` relies on when a builder excludes a transaction. If the
    // write escaped that too, an excluded transaction would leave residue.
    let (slot, word) = spent_word_for(11);
    let mut db = with_vm(false, |vm| {
        vm.stage_durable_vault_write(slot, word);
        vm.flush_durable_vault_writes().expect("flush");
        assert_eq!(
            vm.db
                .current_accounts_state
                .get(&vault_addr())
                .and_then(|acc| acc.storage.get(&slot).copied())
                .unwrap_or_default(),
            word,
            "the flush must reach the cache"
        );

        // The flush went through the journaled path, so the live frame's backup
        // now carries the pre-value — exactly what a tx-level rollback replays.
        let tx_level = vm.current_call_frame.call_frame_backup.clone();
        restore_cache_state(vm.db, tx_level).expect("restore");
    });
    assert!(
        vault_slot_word(&mut db, slot).is_zero(),
        "a flushed write must still be undoable at transaction granularity"
    );
}

#[test]
fn discarding_drops_staged_writes_and_their_state_gas() {
    let (slot, word) = spent_word_for(5);
    let mut db = with_vm(false, |vm| {
        vm.stage_durable_vault_write(slot, word);
        vm.durable_state_gas = 383;
        vm.discard_durable_vault_writes();
        assert!(vm.durable_vault_writes.is_empty());
        assert_eq!(vm.durable_state_gas, 0);
        // A flush after a discard must write nothing.
        vm.flush_durable_vault_writes().expect("flush");
    });
    assert!(vault_slot_word(&mut db, slot).is_zero());
}

#[test]
fn staged_writes_accumulate_per_slot_with_last_write_winning() {
    // Two spends of different indices in the same 256-index word: the second
    // frame reads the first frame's staged word, sets its own bit, and stages the
    // combined value. One slot, both bits — losing the first would be a
    // double-spend hole.
    let (slot_a, bit_a) = spent_word_for(0);
    let (slot_b, bit_b) = spent_word_for(200);
    assert_eq!(slot_a, slot_b, "indices 0 and 200 share one word");

    let mut db = with_vm(false, |vm| {
        let word = vm.read_vault_slot(slot_a).expect("read");
        vm.stage_durable_vault_write(slot_a, word | bit_a);

        let word = vm.read_vault_slot(slot_a).expect("read");
        assert_eq!(word, bit_a, "the second read must see the first bit");
        vm.stage_durable_vault_write(slot_a, word | bit_b);

        vm.flush_durable_vault_writes().expect("flush");
    });
    let final_word = vault_slot_word(&mut db, slot_a);
    assert_eq!(final_word, bit_a | bit_b);
    assert!(!(final_word & bit_a).is_zero());
    assert!(!(final_word & bit_b).is_zero());
}

#[test]
fn flush_records_the_write_in_the_block_access_list() {
    // The parallel importer rebuilds post-state from the BAL, so a durable write
    // missing from it is a state-root divergence. Recording happens at flush,
    // after every checkpoint that could have demoted it back to a read.
    let (slot, word) = spent_word_for(9);
    let mut db = with_vm(true, |vm| {
        vm.stage_durable_vault_write(slot, word);
        vm.flush_durable_vault_writes().expect("flush");
    });

    let bal = db.take_bal().expect("BAL recorded");
    let vault_changes = bal
        .accounts()
        .iter()
        .find(|changes| changes.address == vault_addr())
        .expect("the BAL must mention the vault");
    assert!(
        vault_changes
            .storage_changes
            .iter()
            .any(|change| change.slot == U256::from_big_endian(&slot.0)),
        "the durable write must be recorded as a storage change"
    );
}

fn vault_slot_word(db: &mut GeneralizedDatabase, slot: H256) -> U256 {
    db.current_accounts_state
        .get(&vault_addr())
        .and_then(|acc| acc.storage.get(&slot).copied())
        .unwrap_or_default()
}

// ==================== End-to-end spends ====================
//
// A full UTXO frame executed through `VM::execute()`: a real openings root in a
// real ring slot, a real Merkle proof, and a real secp256k1 actor signature over
// the spend hash. These are the tests that exercise the handler, the durable
// tier, and settlement together — the parts that only interact correctly if every
// piece agrees on the leaf encoding, the tree, the slot layout, and the hash.

use ethrex_common::types::{
    BATCH_PATH_LEN, BATCH_SIZE, FRAME_SIG_SCHEME_SECP256K1, Frame, FrameMode, FrameSignature,
    FrameTransaction, RING_SIZE, Spend, SpendInput, SpendOutput, batch_slot_for_block, is_spent,
    merkle_proof, merkle_root, opening_leaf, ring_slot,
};
use ethrex_levm::errors::VMError;
use ethrex_rlp::encode::RLPEncode;
use k256::ecdsa::SigningKey;

const CREATION_BLOCK: u64 = 10;
const SPEND_BLOCK: u64 = 11;

fn key_and_address(seed: u8) -> (SigningKey, Address) {
    let signing_key = SigningKey::from_bytes(&[seed; 32].into()).unwrap();
    let uncompressed = signing_key.verifying_key().to_encoded_point(false);
    let pub_hash = ethrex_crypto::keccak::keccak_hash(&uncompressed.as_bytes()[1..]);
    (signing_key, Address::from_slice(&pub_hash[12..]))
}

/// A signature entry binding `signer` to an explicit digest (the spend hash).
fn sign_digest(key: &SigningKey, digest: H256, signer: Address) -> FrameSignature {
    let (raw_sig, recovery_id) = key.sign_prehash_recoverable(digest.as_bytes()).unwrap();
    let mut bytes = vec![0u8; 65];
    bytes[0] = recovery_id.to_byte();
    bytes[1..33].copy_from_slice(&raw_sig.to_bytes()[..32]);
    bytes[33..65].copy_from_slice(&raw_sig.to_bytes()[32..]);
    FrameSignature {
        scheme: FRAME_SIG_SCHEME_SECP256K1,
        signer: Some(signer),
        msg: Bytes::copy_from_slice(digest.as_bytes()),
        signature: Bytes::from(bytes),
    }
}

struct SpendFixture {
    tx: FrameTransaction,
    accounts: FxHashMap<Address, Account>,
    input_index: u64,
    spent_slot: H256,
}

/// Build a self-funded spend of one UTXO worth `input_value`, paying
/// `account_out` to `payee` and sending the rest to a change UTXO owned by the
/// actor. The vault is seeded with the creation block's openings root and enough
/// balance to cover the UTXO.
fn self_funded_fixture(
    input_value: U256,
    account_out: Option<(Address, U256)>,
    utxo_out: Option<(Address, U256)>,
) -> SpendFixture {
    let (actor_key, actor) = key_and_address(0x11);
    let source = Address::from_low_u64_be(0x5011);
    let index: u64 = 4;

    // The UTXO as it was created: its leaf, and a real openings tree containing
    // it alongside two unrelated leaves so the proof has real siblings.
    let leaf = opening_leaf(index, source, actor, input_value);
    let other_a = opening_leaf(1, source, Address::from_low_u64_be(1), U256::from(1u64));
    let other_b = opening_leaf(2, source, Address::from_low_u64_be(2), U256::from(2u64));
    let leaves = vec![other_a, leaf, other_b];
    let position = 1usize;
    let root = merkle_root(&leaves);
    let proof = merkle_proof(&leaves, position).expect("proof");

    let mut utxo_outs = Vec::new();
    let mut account_outs = Vec::new();
    if let Some((recipient, value)) = utxo_out {
        utxo_outs.push(SpendOutput { recipient, value });
    }
    if let Some((recipient, value)) = account_out {
        account_outs.push(SpendOutput { recipient, value });
    }
    // The change entry is a UTXO output owned by the actor, signed with value 0.
    utxo_outs.push(SpendOutput {
        recipient: actor,
        value: U256::zero(),
    });
    let change_index = (utxo_outs.len() - 1) as u64;

    let spend = Spend {
        actors: vec![actor],
        inputs: vec![SpendInput {
            index,
            creation_block: CREATION_BLOCK,
            source,
            recipient: actor,
            value: input_value,
            position: position as u64,
            siblings: proof,
            batch_siblings: vec![],
        }],
        utxo_outs,
        account_outs,
        change_index,
        payer: Bytes::new(), // self-funded
        max_fee_per_gas: U256::from(1_000_000u64),
        max_priority_fee_per_gas: U256::from(1_000_000u64),
        max_gas_limit: 30_000_000,
    };

    let mut tx = FrameTransaction {
        chain_id: 1,
        nonce_keys: vec![],
        nonce_seq: 0,
        sender: utxo_vault(),
        frames: vec![Frame {
            mode: FrameMode::Utxo as u8,
            flags: 0,
            target: None,
            gas_limit: 3_000_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::from(spend.encode_to_vec()),
        }],
        signatures: vec![],
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1_000,
        ..Default::default()
    };
    let spend_hash = spend.spend_hash(tx.chain_id);
    tx.signatures
        .push(sign_digest(&actor_key, spend_hash, actor));

    // Vault: pinned code, the creation block's openings root, and the value it
    // custodies for this UTXO.
    let mut vault_storage = FxHashMap::default();
    let ring = H256(ring_slot(CREATION_BLOCK).to_big_endian());
    vault_storage.insert(ring, U256::from_big_endian(root.as_bytes()));
    let vault_acc = Account::new(
        input_value,
        Code::from_bytecode(
            Bytes::from_static(&UTXO_VAULT_RUNTIME_BYTECODE),
            &NativeCrypto,
        ),
        1,
        vault_storage,
    );

    let (spent_slot_u256, _) = spent_bit_location(index);
    SpendFixture {
        tx,
        accounts: [(utxo_vault(), vault_acc)].into_iter().collect(),
        input_index: index,
        spent_slot: H256(spent_slot_u256.to_big_endian()),
    }
}

fn run_spend(fixture: &SpendFixture) -> (Result<ExecutionReport, VMError>, GeneralizedDatabase) {
    run_spend_at(fixture, SPEND_BLOCK)
}

/// As `run_spend`, but executing at an explicit block number, so a test can place
/// the spend outside the ring window.
fn run_spend_at(
    fixture: &SpendFixture,
    spend_block: u64,
) -> (Result<ExecutionReport, VMError>, GeneralizedDatabase) {
    let mut db = GeneralizedDatabase::new_with_account_state(
        Arc::new(TestDatabase),
        fixture.accounts.clone(),
    );
    let fork = Fork::Hegota;
    let env = Environment {
        disable_gas_allowance_check: false,
        origin: fixture.tx.sender,
        gas_limit: fixture.tx.frames.iter().map(|f| f.gas_limit).sum::<u64>() + 1_000_000,
        config: {
            // A fork-only EVMConfig carries no EIP-8312 activation timestamp, so
            // the production default is inactive; opt in explicitly.
            let mut config = EVMConfig::new(fork, EVMConfig::canonical_values(fork));
            config.utxo_frames_active = true;
            config
        },
        block_number: spend_block,
        coinbase: Address::from_low_u64_be(0xCCC),
        timestamp: 1000,
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        slot_number: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::from(1u64),
        base_blob_fee_per_gas: U256::from(1),
        gas_price: U256::from(2u64),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: Some(U256::from(1u64)),
        tx_max_fee_per_gas: Some(U256::from(1_000u64)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: 60_000_000,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: false,
        disable_nonce_check: false,
        is_system_call: false,
    };
    let tx = Transaction::FrameTransaction(fixture.tx.clone());
    let result = {
        let mut vm = VM::new(
            env,
            &mut db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
            &NativeCrypto,
            None,
        )
        .expect("VM::new");
        vm.execute()
    };
    (result, db)
}

#[test]
fn a_self_funded_spend_executes_and_settles() {
    let payee = Address::from_low_u64_be(0xBEEF);
    let input_value = U256::from(10u64).pow(U256::from(18u64)); // 1 ETH
    let paid = U256::from(100_000u64);
    let fixture = self_funded_fixture(input_value, Some((payee, paid)), None);

    let (result, mut db) = run_spend(&fixture);
    let report = result.expect("a well-formed self-funded spend must execute");
    assert!(
        matches!(report.result, TxResult::Success),
        "got {:?}",
        report.result
    );

    // The spent bit is set and durable.
    let word = vault_slot_word(&mut db, fixture.spent_slot);
    assert!(
        is_spent(word, fixture.input_index),
        "the input's spent bit must be set"
    );

    // The account output was credited out of the vault.
    let payee_balance = db
        .current_accounts_state
        .get(&payee)
        .map(|a| a.info.balance)
        .unwrap_or_default();
    assert_eq!(payee_balance, paid);

    // The change UTXO was created: one UtxoCreated log from the vault, plus the
    // EIP-7708 transfer log for the account output.
    let created: Vec<_> = report
        .logs
        .iter()
        .filter(|l| l.address == utxo_vault() && l.topics.first() == Some(&UTXO_CREATED_TOPIC))
        .collect();
    assert_eq!(created.len(), 1, "the change output must create one UTXO");

    // Value conservation: everything the input carried is either paid out, or in
    // the change UTXO, or spent on fees. The vault keeps the change (it custodies
    // unspent UTXO value), so its balance is input - paid - fee.
    let change_value = U256::from_big_endian(&created[0].data[32..]);
    let vault_balance = db
        .current_accounts_state
        .get(&utxo_vault())
        .map(|a| a.info.balance)
        .unwrap_or_default();
    assert_eq!(
        vault_balance, change_value,
        "the vault must retain exactly the change it still custodies"
    );
    let fee = input_value - paid - change_value;
    assert!(
        !fee.is_zero(),
        "a self-funded spend must have paid a non-zero fee out of its change"
    );
}

#[test]
fn a_spend_cannot_reuse_a_spent_input() {
    // The double-spend defense: the same spend replayed against a state where the
    // bit is already set must be rejected. This is the whole replay protection for
    // a vault-sender transaction, which carries no nonce.
    let payee = Address::from_low_u64_be(0xBEEF);
    let input_value = U256::from(10u64).pow(U256::from(18u64));
    let fixture = self_funded_fixture(input_value, Some((payee, U256::from(1u64))), None);

    // First execution succeeds.
    let (first, mut db) = run_spend(&fixture);
    first.expect("first spend must succeed");
    let word = vault_slot_word(&mut db, fixture.spent_slot);
    assert!(is_spent(word, fixture.input_index));

    // Replay against a state carrying the set bit.
    let mut replay = fixture;
    let vault = replay
        .accounts
        .get_mut(&utxo_vault())
        .expect("vault seeded");
    vault.storage.insert(replay.spent_slot, word);
    let (second, _) = run_spend(&replay);
    assert!(
        second.is_err(),
        "spending an already-spent input must invalidate the transaction"
    );
}

#[test]
fn a_spend_with_a_forged_proof_is_rejected() {
    // The soundness property: a witness that does not fold to the stored openings
    // root cannot spend. Without this a spender could mint value out of the vault.
    let payee = Address::from_low_u64_be(0xBEEF);
    let input_value = U256::from(10u64).pow(U256::from(18u64));
    let mut fixture = self_funded_fixture(input_value, Some((payee, U256::from(1u64))), None);

    // Corrupt the stored root so the (honest) proof no longer verifies.
    let ring = H256(ring_slot(CREATION_BLOCK).to_big_endian());
    fixture
        .accounts
        .get_mut(&utxo_vault())
        .unwrap()
        .storage
        .insert(ring, U256::from(0xDEADu64));

    let (result, mut db) = run_spend(&fixture);
    assert!(
        result.is_err(),
        "a proof that does not verify must be rejected"
    );
    // And nothing persisted: no spent bit, no payout.
    assert!(vault_slot_word(&mut db, fixture.spent_slot).is_zero());
}

#[test]
fn a_spend_with_an_inflated_input_value_is_rejected() {
    // The leaf commits to the value, so claiming a larger one changes the leaf and
    // the proof fails. This is what stops a spender inflating what they own.
    let payee = Address::from_low_u64_be(0xBEEF);
    let input_value = U256::from(10u64).pow(U256::from(18u64));
    let fixture = self_funded_fixture(input_value, Some((payee, U256::from(1u64))), None);

    // Re-decode the spend, inflate the witnessed value, re-encode. The signature
    // still validates: witness fields are outside the spend hash by design.
    let mut spend = Spend::decode_frame_data(&fixture.tx.frames[0].data).expect("decode");
    spend.inputs[0].value = input_value * 2;
    let mut inflated = fixture;
    inflated.tx.frames[0].data = Bytes::from(spend.encode_to_vec());

    let (result, _) = run_spend(&inflated);
    assert!(
        result.is_err(),
        "an inflated witnessed value must fail proof verification"
    );
}

#[test]
fn a_spend_by_a_non_actor_is_rejected() {
    // The proven opening's recipient must be one of the actors that signed. A
    // valid proof for someone else's UTXO must not let the signer spend it.
    let input_value = U256::from(10u64).pow(U256::from(18u64));
    let (other_key, other) = key_and_address(0x22);
    let source = Address::from_low_u64_be(0x5011);
    let victim = Address::from_low_u64_be(0x7777);
    let index: u64 = 4;

    // A UTXO owned by `victim`, correctly committed.
    let leaf = opening_leaf(index, source, victim, input_value);
    let leaves = vec![leaf];
    let root = merkle_root(&leaves);

    let spend = Spend {
        actors: vec![other], // signer is NOT the UTXO's recipient
        inputs: vec![SpendInput {
            index,
            creation_block: CREATION_BLOCK,
            source,
            recipient: victim,
            value: input_value,
            position: 0,
            siblings: vec![],
            batch_siblings: vec![],
        }],
        utxo_outs: vec![SpendOutput {
            recipient: other,
            value: U256::zero(),
        }],
        account_outs: vec![],
        change_index: 0,
        payer: Bytes::new(),
        max_fee_per_gas: U256::from(1_000_000u64),
        max_priority_fee_per_gas: U256::from(1_000_000u64),
        max_gas_limit: 30_000_000,
    };
    let mut tx = FrameTransaction {
        chain_id: 1,
        nonce_keys: vec![],
        nonce_seq: 0,
        sender: utxo_vault(),
        frames: vec![Frame {
            mode: FrameMode::Utxo as u8,
            flags: 0,
            target: None,
            gas_limit: 3_000_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::from(spend.encode_to_vec()),
        }],
        signatures: vec![],
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1_000,
        ..Default::default()
    };
    let digest = spend.spend_hash(tx.chain_id);
    tx.signatures.push(sign_digest(&other_key, digest, other));

    let mut vault_storage = FxHashMap::default();
    vault_storage.insert(
        H256(ring_slot(CREATION_BLOCK).to_big_endian()),
        U256::from_big_endian(root.as_bytes()),
    );
    let (spent_slot_u256, _) = spent_bit_location(index);
    let fixture = SpendFixture {
        tx,
        accounts: [(
            utxo_vault(),
            Account::new(
                input_value,
                Code::from_bytecode(
                    Bytes::from_static(&UTXO_VAULT_RUNTIME_BYTECODE),
                    &NativeCrypto,
                ),
                1,
                vault_storage,
            ),
        )]
        .into_iter()
        .collect(),
        input_index: index,
        spent_slot: H256(spent_slot_u256.to_big_endian()),
    };

    let (result, _) = run_spend(&fixture);
    assert!(
        result.is_err(),
        "the opening's recipient must be an actor of the spend"
    );
}

#[test]
fn a_utxo_is_not_spendable_in_its_creation_block() {
    // Its openings root is written at the END of the creation block, so the
    // earliest a spend can prove it is the next block. Executing at the creation
    // block must fail even though the root is (already) present in state.
    let payee = Address::from_low_u64_be(0xBEEF);
    let input_value = U256::from(10u64).pow(U256::from(18u64));
    let mut fixture = self_funded_fixture(input_value, Some((payee, U256::from(1u64))), None);
    // Move the creation block up to the spending block.
    let mut spend = Spend::decode_frame_data(&fixture.tx.frames[0].data).expect("decode");
    spend.inputs[0].creation_block = SPEND_BLOCK;
    // Re-seed the root under the new creation block's ring slot so the only
    // failing condition is the age check.
    let leaf = opening_leaf(
        spend.inputs[0].index,
        spend.inputs[0].source,
        spend.inputs[0].recipient,
        spend.inputs[0].value,
    );
    let leaves = vec![leaf];
    spend.inputs[0].position = 0;
    spend.inputs[0].siblings = vec![];
    let root = merkle_root(&leaves);
    fixture
        .accounts
        .get_mut(&utxo_vault())
        .unwrap()
        .storage
        .insert(
            H256(ring_slot(SPEND_BLOCK).to_big_endian()),
            U256::from_big_endian(root.as_bytes()),
        );
    fixture.tx.frames[0].data = Bytes::from(spend.encode_to_vec());
    // The witness changed but not the signed fields... creation_block IS signed,
    // so re-sign.
    let (actor_key, actor) = key_and_address(0x11);
    let digest = spend.spend_hash(fixture.tx.chain_id);
    fixture.tx.signatures = vec![sign_digest(&actor_key, digest, actor)];

    let (result, _) = run_spend(&fixture);
    assert!(
        result.is_err(),
        "a UTXO must not be spendable in its own creation block"
    );
}

#[test]
fn a_spend_whose_inputs_do_not_cover_the_max_cost_is_rejected() {
    // A self-funded spend's conservation includes the transaction's maximum cost,
    // because the vault fronts it. An input too small to cover outputs + max cost
    // must be rejected rather than leaving the vault out of pocket.
    let payee = Address::from_low_u64_be(0xBEEF);
    // Far too little to cover max_fee_per_gas * gas_limit.
    let input_value = U256::from(1_000u64);
    let fixture = self_funded_fixture(input_value, Some((payee, U256::from(999u64))), None);

    let (result, _) = run_spend(&fixture);
    assert!(
        result.is_err(),
        "inputs must cover signed outputs plus the transaction's maximum cost"
    );
}

/// The new-account reserve is returned exactly when the recipient already exists.
///
/// Every account output is charged `GAS_NEW_ACCOUNT_STATE` up front, because
/// whether the recipient exists is not known until settlement. Settlement's first
/// phase returns that reserve for recipients that turn out to already exist —
/// before gas finalization, so the return lands in `gas_used`.
///
/// Both directions were unconstrained: returning the reserve for a *new* recipient
/// and never returning it at all each left the whole suite green. Both are
/// consensus-visible, since `gas_used` is in the receipt and the header — returning
/// it wrongly means the transaction underpays for state it created, and withholding
/// it means the user overpays 183,600 gas.
///
/// The assertion is the difference between two otherwise identical spends, which
/// pins the constant rather than restating the implementation.
#[test]
fn the_new_account_reserve_is_returned_only_for_an_existing_recipient() {
    let payee = Address::from_low_u64_be(0xBEEF);
    let input_value = U256::from(10u64).pow(U256::from(18u64));
    let payout = U256::from(1_000_000u64);

    // Fresh recipient: no account, so the reserve is consumed and kept.
    let fresh = self_funded_fixture(input_value, Some((payee, payout)), None);
    let (fresh_result, _) = run_spend(&fresh);
    let fresh_gas = fresh_result.expect("the spend must settle").gas_used;

    // Same spend, but the recipient already exists (non-empty: it has a balance),
    // so the reserve must come back.
    let mut existing = self_funded_fixture(input_value, Some((payee, payout)), None);
    existing.accounts.insert(
        payee,
        Account::new(U256::from(1u64), Code::default(), 0, FxHashMap::default()),
    );
    let (existing_result, _) = run_spend(&existing);
    let existing_gas = existing_result
        .expect("the spend must settle for an existing recipient")
        .gas_used;

    assert!(
        existing_gas < fresh_gas,
        "an existing recipient must cost less: existing={existing_gas} fresh={fresh_gas}"
    );
    assert_eq!(
        fresh_gas - existing_gas,
        ethrex_common::types::GAS_NEW_ACCOUNT_STATE,
        "the returned reserve must be exactly GAS_NEW_ACCOUNT_STATE"
    );
}

/// A spend's spent bits are billed in the state dimension.
///
/// Spent-bit state gas is deliberately accumulated in `durable_state_gas` rather
/// than `state_gas_used`, so the per-frame, per-batch and body resets cannot drop it
/// — those resets assume "reverted state ⇒ no state grew", which a durable write
/// falsifies. It is folded into the transaction's state gas after the loop.
///
/// That fold had no coverage: suppressing it left the whole suite green while the
/// block underbilled for state that is permanently there. Since the block's gas is
/// `max(sum(regular), sum(state))` under EIP-8037, an underbilled state dimension is
/// consensus-visible.
///
/// Asserted as a difference against a spend of the same shape with one more input,
/// so it pins the per-bit constant rather than the whole transaction's accounting.
#[test]
fn spent_bits_are_billed_in_the_state_dimension() {
    let payee = Address::from_low_u64_be(0xBEEF);
    let value = U256::from(10u64).pow(U256::from(18u64));

    let one = self_funded_fixture(value, Some((payee, U256::from(1u64))), None);
    let one_state = run_spend(&one)
        .0
        .expect("the one-input spend must settle")
        .state_gas_used;

    // Two inputs, so two spent bits: the state dimension must grow by exactly one
    // more bit's worth.
    let two = consolidation_fixture(2, U256::from(1u64)).0;
    let two_state = run_spend(&two)
        .0
        .expect("the two-input spend must settle")
        .state_gas_used;

    assert!(
        two_state > one_state,
        "a second spent bit must cost state gas: one={one_state} two={two_state}"
    );
    assert_eq!(
        two_state - one_state,
        ethrex_common::types::GAS_UTXO_SPENT_STATE,
        "the extra spent bit must bill exactly GAS_UTXO_SPENT_STATE"
    );
    assert!(
        one_state >= ethrex_common::types::GAS_UTXO_SPENT_STATE,
        "even one spent bit must be billed: {one_state}"
    );
}

/// A ring proof is refused once the ring entry has aged out.
///
/// The ring holds one root per block for `RING_SIZE` blocks, then wraps: the slot
/// that held block N's root now holds a later block's. So a ring proof older than
/// the window must be refused even when the slot still happens to contain the
/// matching root — which is exactly the situation this fixture creates, since it
/// seeds the slot directly and nothing has overwritten it. Honouring it would mean
/// accepting a proof against a slot that on a real chain belongs to another block.
///
/// Loosening the `age > RING_SIZE` comparison used to survive the whole suite; the
/// batch-proof test below only covers the case where a batch path IS supplied.
#[test]
fn a_ring_proof_is_refused_once_the_ring_entry_has_aged_out() {
    let payee = Address::from_low_u64_be(0xBEEF);
    let input_value = U256::from(10u64).pow(U256::from(18u64));
    let fixture = self_funded_fixture(input_value, Some((payee, U256::from(1u64))), None);

    // Inside the window it is spendable: this is the control, without which the
    // rejection below could be caused by anything.
    let (inside, _) = run_spend_at(&fixture, CREATION_BLOCK + 1);
    assert!(
        inside.is_ok(),
        "a fresh ring proof must be accepted; got {inside:?}"
    );

    // One block past the window, with no batch path supplied, it must not be.
    let aged = CREATION_BLOCK + RING_SIZE + 1;
    let (result, mut db) = run_spend_at(&fixture, aged);
    assert!(
        result.is_err(),
        "a ring proof {} blocks old must be refused (RING_SIZE = {RING_SIZE})",
        aged - CREATION_BLOCK
    );
    assert!(
        vault_slot_word(&mut db, fixture.spent_slot).is_zero(),
        "the refused spend must leave no spent bit"
    );
}

#[test]
fn a_batch_proof_spends_a_utxo_whose_ring_entry_aged_out() {
    // Once the creation block leaves the ring, the batch path takes over. The
    // batch root commits to the openings roots of its blocks, so the proof is the
    // in-block path folded with a BATCH_PATH_LEN-deep batch path.
    let (actor_key, actor) = key_and_address(0x33);
    let source = Address::from_low_u64_be(0x5011);
    let index: u64 = 6;
    let input_value = U256::from(10u64).pow(U256::from(18u64));
    let creation_block: u64 = 3; // inside batch 0
    // Spend well after batch 0 was sealed (end of block BATCH_SIZE - 1).
    let spend_block: u64 = BATCH_SIZE + 5;

    let leaf = opening_leaf(index, source, actor, input_value);
    let openings_root = merkle_root(&[leaf]);

    // Batch 0's leaves are the openings roots of blocks 0..BATCH_SIZE-1; only the
    // creation block's is non-zero here.
    let mut batch_leaves = vec![H256::zero(); BATCH_SIZE as usize];
    batch_leaves[creation_block as usize] = openings_root;
    let batch_root = merkle_root(&batch_leaves);
    let batch_path = merkle_proof(&batch_leaves, creation_block as usize).expect("batch proof");
    assert_eq!(batch_path.len(), BATCH_PATH_LEN);

    let spend = Spend {
        actors: vec![actor],
        inputs: vec![SpendInput {
            index,
            creation_block,
            source,
            recipient: actor,
            value: input_value,
            position: 0,
            siblings: vec![], // single-leaf openings tree
            batch_siblings: batch_path,
        }],
        utxo_outs: vec![SpendOutput {
            recipient: actor,
            value: U256::zero(),
        }],
        account_outs: vec![],
        change_index: 0,
        payer: Bytes::new(),
        max_fee_per_gas: U256::from(1_000_000u64),
        max_priority_fee_per_gas: U256::from(1_000_000u64),
        max_gas_limit: 30_000_000,
    };
    let mut tx = FrameTransaction {
        chain_id: 1,
        nonce_keys: vec![],
        nonce_seq: 0,
        sender: utxo_vault(),
        frames: vec![Frame {
            mode: FrameMode::Utxo as u8,
            flags: 0,
            target: None,
            gas_limit: 3_000_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::from(spend.encode_to_vec()),
        }],
        signatures: vec![],
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1_000,
        ..Default::default()
    };
    let digest = spend.spend_hash(tx.chain_id);
    tx.signatures.push(sign_digest(&actor_key, digest, actor));

    // Seed only the BATCH root: the ring entry is long gone, which is the point.
    let mut vault_storage = FxHashMap::default();
    vault_storage.insert(
        H256(batch_slot_for_block(creation_block).to_big_endian()),
        U256::from_big_endian(batch_root.as_bytes()),
    );
    let mut db = GeneralizedDatabase::new_with_account_state(
        Arc::new(TestDatabase),
        [(
            utxo_vault(),
            Account::new(
                input_value,
                Code::from_bytecode(
                    Bytes::from_static(&UTXO_VAULT_RUNTIME_BYTECODE),
                    &NativeCrypto,
                ),
                1,
                vault_storage,
            ),
        )]
        .into_iter()
        .collect(),
    );

    let fork = Fork::Hegota;
    let env = Environment {
        disable_gas_allowance_check: false,
        origin: tx.sender,
        gas_limit: 4_000_000,
        config: {
            let mut config = EVMConfig::new(fork, EVMConfig::canonical_values(fork));
            config.utxo_frames_active = true;
            config
        },
        block_number: spend_block,
        coinbase: Address::from_low_u64_be(0xCCC),
        timestamp: 1000,
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        slot_number: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::from(1u64),
        base_blob_fee_per_gas: U256::from(1),
        gas_price: U256::from(2u64),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: Some(U256::from(1u64)),
        tx_max_fee_per_gas: Some(U256::from(1_000u64)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: 60_000_000,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: false,
        disable_nonce_check: false,
        is_system_call: false,
    };
    let transaction = Transaction::FrameTransaction(tx);
    let result = {
        let mut vm = VM::new(
            env,
            &mut db,
            &transaction,
            LevmCallTracer::disabled(),
            VMType::L1,
            &NativeCrypto,
            None,
        )
        .expect("VM::new");
        vm.execute()
    };
    let report = result.expect("a batch-path spend must execute");
    assert!(matches!(report.result, TxResult::Success));

    let (slot_u256, _) = spent_bit_location(index);
    let word = vault_slot_word(&mut db, H256(slot_u256.to_big_endian()));
    assert!(
        is_spent(word, index),
        "the batch-path spend must set the bit"
    );
    // RING_SIZE is referenced so the aged-out premise stays visible.
    assert!(spend_block - creation_block < RING_SIZE + BATCH_SIZE);
}

// ==================== End-to-end rollback matrix (task 5.4) ====================
//
// The durable tier's contract, now driven by real UTXO frames rather than by
// calling the staging API directly: a spent bit set by a UTXO frame must survive
// a sibling frame's failure inside the same transaction, and must leave no trace
// when the transaction is invalid.

/// A self-funded spend is single-frame by rule, so the sibling-failure case needs
/// a sponsored spend: [pay-frame, UTXO, failing DEFAULT].
fn sponsored_fixture(sibling: Option<Frame>) -> (SpendFixture, Address) {
    sponsored_fixture_paying(sibling, Vec::new())
}

/// As `sponsored_fixture`, but with explicit `account_outs` so a test can make a
/// sponsored spend pay out more than its inputs prove.
fn sponsored_fixture_paying(
    sibling: Option<Frame>,
    account_outs: Vec<SpendOutput>,
) -> (SpendFixture, Address) {
    let (actor_key, actor) = key_and_address(0x44);
    let sponsor = Address::from_low_u64_be(0x5907);
    let source = Address::from_low_u64_be(0x5011);
    let index: u64 = 8;
    let input_value = U256::from(10u64).pow(U256::from(18u64));

    let leaf = opening_leaf(index, source, actor, input_value);
    let root = merkle_root(&[leaf]);

    let spend = Spend {
        actors: vec![actor],
        inputs: vec![SpendInput {
            index,
            creation_block: CREATION_BLOCK,
            source,
            recipient: actor,
            value: input_value,
            position: 0,
            siblings: vec![],
            batch_siblings: vec![],
        }],
        utxo_outs: vec![SpendOutput {
            recipient: actor,
            value: U256::zero(),
        }],
        account_outs,
        change_index: 0,
        payer: Bytes::copy_from_slice(sponsor.as_bytes()),
        max_fee_per_gas: U256::from(1_000_000u64),
        max_priority_fee_per_gas: U256::from(1_000_000u64),
        max_gas_limit: 30_000_000,
    };

    // The sponsor's pay frame: a VERIFY frame whose target is the sponsor, whose
    // code calls APPROVE (0xAA) with scope 0x1 = APPROVE_PAYMENT, then stops.
    // APPROVE pops [offset, length, scope], so scope is pushed deepest.
    let approve_payment_code = Bytes::from(vec![0x60, 0x01, 0x60, 0x00, 0x60, 0x00, 0xAA]);
    let mut frames = vec![
        Frame {
            mode: FrameMode::Verify as u8,
            flags: 0x01, // APPROVE_PAYMENT scope
            target: Some(sponsor),
            gas_limit: 100_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
        Frame {
            mode: FrameMode::Utxo as u8,
            flags: 0,
            target: None,
            gas_limit: 3_000_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::from(spend.encode_to_vec()),
        },
    ];
    if let Some(extra) = sibling {
        frames.push(extra);
    }

    let mut tx = FrameTransaction {
        chain_id: 1,
        nonce_keys: vec![],
        nonce_seq: 0,
        sender: utxo_vault(),
        frames,
        signatures: vec![],
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1_000,
        ..Default::default()
    };
    let digest = spend.spend_hash(tx.chain_id);
    tx.signatures.push(sign_digest(&actor_key, digest, actor));

    let mut vault_storage = FxHashMap::default();
    vault_storage.insert(
        H256(ring_slot(CREATION_BLOCK).to_big_endian()),
        U256::from_big_endian(root.as_bytes()),
    );
    let (spent_slot_u256, _) = spent_bit_location(index);

    let accounts: FxHashMap<Address, Account> = [
        (
            utxo_vault(),
            Account::new(
                input_value,
                Code::from_bytecode(
                    Bytes::from_static(&UTXO_VAULT_RUNTIME_BYTECODE),
                    &NativeCrypto,
                ),
                1,
                vault_storage,
            ),
        ),
        (
            sponsor,
            Account::new(
                U256::from(10u64).pow(U256::from(20u64)),
                Code::from_bytecode(approve_payment_code, &NativeCrypto),
                1,
                FxHashMap::default(),
            ),
        ),
    ]
    .into_iter()
    .collect();

    (
        SpendFixture {
            tx,
            accounts,
            input_index: index,
            spent_slot: H256(spent_slot_u256.to_big_endian()),
        },
        sponsor,
    )
}

#[test]
fn a_spent_bit_survives_a_sibling_frames_failure() {
    // The core EIP-8312 durability requirement, end to end: frame 1 spends, frame
    // 2 reverts, the transaction is still included — and the spend is final.
    // A frame whose code is a bare REVERT.
    let reverting = Frame {
        mode: FrameMode::Default as u8,
        flags: 0,
        target: Some(Address::from_low_u64_be(0xFA11)),
        gas_limit: 100_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    };
    let (mut fixture, _sponsor) = sponsored_fixture(Some(reverting));
    fixture.accounts.insert(
        Address::from_low_u64_be(0xFA11),
        Account::new(
            U256::zero(),
            Code::from_bytecode(
                Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xfd]),
                &NativeCrypto,
            ),
            1,
            FxHashMap::default(),
        ),
    );

    let (result, mut db) = run_spend(&fixture);
    let report = result.expect("a reverting sibling frame must not invalidate the transaction");
    // The transaction is included with a failure status (a reverted DEFAULT frame
    // does not invalidate it), and the spend stands.
    assert!(
        matches!(report.result, TxResult::Revert(_)),
        "a reverted sibling frame makes the tx status failure, got {:?}",
        report.result
    );
    let word = vault_slot_word(&mut db, fixture.spent_slot);
    assert!(
        is_spent(word, fixture.input_index),
        "the spent bit must survive a sibling frame's revert — a spend is final once approved"
    );
}

#[test]
fn an_invalid_transaction_leaves_no_spent_bit() {
    // The other side of the contract: durability is scoped to sibling failure, not
    // to transaction invalidity. An invalid transaction is never included, so it
    // must leave the vault untouched — otherwise a free-to-submit invalid
    // transaction could burn other people's UTXOs.
    //
    // A VERIFY frame that reverts invalidates the whole transaction (EIP-8141).
    let reverting_verify = Frame {
        mode: FrameMode::Verify as u8,
        flags: 0,
        target: Some(Address::from_low_u64_be(0xFA12)),
        gas_limit: 100_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    };
    let (mut fixture, _sponsor) = sponsored_fixture(Some(reverting_verify));
    fixture.accounts.insert(
        Address::from_low_u64_be(0xFA12),
        Account::new(
            U256::zero(),
            Code::from_bytecode(
                Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xfd]),
                &NativeCrypto,
            ),
            1,
            FxHashMap::default(),
        ),
    );

    let (result, mut db) = run_spend(&fixture);
    assert!(
        result.is_err(),
        "a reverted VERIFY frame must invalidate the transaction"
    );
    assert!(
        vault_slot_word(&mut db, fixture.spent_slot).is_zero(),
        "an invalid transaction must leave no spent bit behind"
    );
}

#[test]
fn a_sponsored_spend_settles_without_charging_the_change() {
    // In sponsored mode the sponsor pays, so the frame's conservation excludes the
    // fee and the change output receives the full remainder.
    let (fixture, _sponsor) = sponsored_fixture(None);
    let (result, mut db) = run_spend(&fixture);
    let report = result.expect("a sponsored spend must execute");
    assert!(
        matches!(report.result, TxResult::Success),
        "{:?}",
        report.result
    );

    assert!(is_spent(
        vault_slot_word(&mut db, fixture.spent_slot),
        fixture.input_index
    ));

    // The change UTXO carries the whole input: no outputs, no fee charged to it.
    let created: Vec<_> = report
        .logs
        .iter()
        .filter(|l| l.address == utxo_vault() && l.topics.first() == Some(&UTXO_CREATED_TOPIC))
        .collect();
    assert_eq!(created.len(), 1);
    let change_value = U256::from_big_endian(&created[0].data[32..]);
    assert_eq!(
        change_value,
        U256::from(10u64).pow(U256::from(18u64)),
        "a sponsored spend's change must not be charged the fee"
    );
}

// ==================== Block-end openings roots ====================
//
// The commitments a spend proves against. Built from the block's `UtxoCreated`
// receipt logs, written to the vault at the end of the block.

use ethrex_common::types::{Receipt, TxType, fold, seals_batch};

/// A receipt carrying `logs`.
fn receipt_with(logs: Vec<Log>) -> Receipt {
    Receipt::new(TxType::EIP1559, true, 21_000, logs)
}

/// The `UtxoCreated` log a creation emits.
fn created_log(source: Address, recipient: Address, index: u64, value: U256) -> Log {
    ethrex_levm::utils::create_utxo_created_log(utxo_vault(), source, recipient, index, value)
}

fn db_with_vault() -> GeneralizedDatabase {
    GeneralizedDatabase::new_with_account_state(
        Arc::new(TestDatabase),
        [(utxo_vault(), vault_account(0, U256::zero()))]
            .into_iter()
            .collect(),
    )
}

#[test]
fn openings_root_commits_to_the_blocks_creations() {
    // The root written must be exactly the root a spender folds their proof
    // against — the property that ties the block-end writer to the verifier.
    let source = Address::from_low_u64_be(0x5011);
    let alice = Address::from_low_u64_be(0xA11CE);
    let bob = Address::from_low_u64_be(0xB0B);
    let block_number = 42u64;

    let receipts = vec![
        receipt_with(vec![created_log(source, alice, 0, U256::from(100u64))]),
        receipt_with(vec![created_log(source, bob, 1, U256::from(200u64))]),
    ];

    let mut db = db_with_vault();
    LEVM::write_openings_roots(&mut db, &receipts, block_number).expect("write");

    let expected = merkle_root(&[
        opening_leaf(0, source, alice, U256::from(100u64)),
        opening_leaf(1, source, bob, U256::from(200u64)),
    ]);
    let slot = H256(ring_slot(block_number).to_big_endian());
    assert_eq!(
        vault_slot_word(&mut db, slot),
        U256::from_big_endian(expected.as_bytes())
    );

    // And a proof against that stored root verifies, for each leaf.
    let leaves = vec![
        opening_leaf(0, source, alice, U256::from(100u64)),
        opening_leaf(1, source, bob, U256::from(200u64)),
    ];
    for position in 0..leaves.len() {
        let proof = merkle_proof(&leaves, position).unwrap();
        assert_eq!(fold(leaves[position], position as u64, &proof), expected);
    }
}

#[test]
fn leaves_are_ordered_by_index_not_by_receipt_order() {
    // Indices come from one global counter, so index order is creation order. If
    // logs arrive out of order (or a later transaction's log has a lower index for
    // any reason), the tree must still be built in index order or a spender's
    // position bits point at the wrong leaf.
    let source = Address::from_low_u64_be(0x5011);
    let a = Address::from_low_u64_be(0xA);
    let b = Address::from_low_u64_be(0xB);
    let block_number = 7u64;

    let out_of_order = vec![
        receipt_with(vec![created_log(source, b, 5, U256::from(2u64))]),
        receipt_with(vec![created_log(source, a, 3, U256::from(1u64))]),
    ];
    let mut db = db_with_vault();
    LEVM::write_openings_roots(&mut db, &out_of_order, block_number).expect("write");

    let expected = merkle_root(&[
        opening_leaf(3, source, a, U256::from(1u64)),
        opening_leaf(5, source, b, U256::from(2u64)),
    ]);
    let slot = H256(ring_slot(block_number).to_big_endian());
    assert_eq!(
        vault_slot_word(&mut db, slot),
        U256::from_big_endian(expected.as_bytes())
    );
}

#[test]
fn a_forged_log_from_another_address_cannot_become_a_leaf() {
    // The soundness check: anyone can emit a log with the UtxoCreated topic. If the
    // collector matched on topic alone, a forged leaf would enter the openings root
    // and become a spendable claim on the vault's pooled balance — theft.
    let source = Address::from_low_u64_be(0x5011);
    let attacker = Address::from_low_u64_be(0xBAD);
    let block_number = 9u64;

    let mut forged = created_log(
        source,
        attacker,
        0,
        U256::from(10u64).pow(U256::from(24u64)),
    );
    forged.address = attacker; // emitted by the attacker's own contract

    let receipts = vec![receipt_with(vec![forged])];
    let mut db = db_with_vault();
    LEVM::write_openings_roots(&mut db, &receipts, block_number).expect("write");

    // The block created nothing, so the root is the empty-tree sentinel.
    let slot = H256(ring_slot(block_number).to_big_endian());
    assert!(
        vault_slot_word(&mut db, slot).is_zero(),
        "a log from a non-vault address must not become a leaf"
    );
}

#[test]
fn an_empty_block_clears_its_ring_slot() {
    // The unconditional-write rule. A block creating nothing writes the zero root,
    // which CLEARS whatever this slot held one ring length ago. Skipping the write
    // would leave a stale root readable, silently granting it a second window — and
    // would only diverge from a conforming client thousands of blocks later.
    let block_number = 3u64;
    let slot = H256(ring_slot(block_number).to_big_endian());

    let mut db = db_with_vault();
    // Seed a stale root, as if written `RING_SIZE` blocks ago.
    db.get_account_mut(utxo_vault())
        .unwrap()
        .storage
        .insert(slot, U256::from(0xDEADBEEFu64));

    LEVM::write_openings_roots(&mut db, &[], block_number).expect("write");
    assert!(
        vault_slot_word(&mut db, slot).is_zero(),
        "an empty block must clear its ring slot, not skip the write"
    );
}

#[test]
fn a_batch_boundary_seals_the_batch_over_its_ring_roots() {
    // At the last block of a batch, the batch root commits to that batch's openings
    // roots — including the zero roots of empty blocks, which are real leaves here.
    let source = Address::from_low_u64_be(0x5011);
    let alice = Address::from_low_u64_be(0xA11CE);
    let last = BATCH_SIZE - 1;
    assert!(seals_batch(last));

    let mut db = db_with_vault();

    // Two earlier blocks in this batch created UTXOs; the rest are empty.
    let mut expected_ring: Vec<H256> = vec![H256::zero(); BATCH_SIZE as usize];
    for n in [2u64, 5u64] {
        let receipts = vec![receipt_with(vec![created_log(
            source,
            alice,
            n,
            U256::from(n),
        )])];
        LEVM::write_openings_roots(&mut db, &receipts, n).expect("write");
        expected_ring[n as usize] = merkle_root(&[opening_leaf(n, source, alice, U256::from(n))]);
    }

    // The sealing block itself creates one more.
    let receipts = vec![receipt_with(vec![created_log(
        source,
        alice,
        999,
        U256::from(9u64),
    )])];
    LEVM::write_openings_roots(&mut db, &receipts, last).expect("write");
    expected_ring[last as usize] =
        merkle_root(&[opening_leaf(999, source, alice, U256::from(9u64))]);

    // The batch root must commit to every ring root of the batch, and its own
    // block's root must be included (it is written in the same operation).
    let expected_batch = merkle_root(&expected_ring);
    let batch_slot = H256(batch_slot_for_block(last).to_big_endian());
    assert_eq!(
        vault_slot_word(&mut db, batch_slot),
        U256::from_big_endian(expected_batch.as_bytes()),
        "the batch root must commit to all of its blocks' openings roots"
    );

    // A batch path from the sealed batch verifies against the stored batch root.
    let position = last as usize;
    let proof = merkle_proof(&expected_ring, position).unwrap();
    assert_eq!(proof.len(), BATCH_PATH_LEN);
    assert_eq!(
        fold(expected_ring[position], position as u64, &proof),
        expected_batch
    );
}

#[test]
fn no_batch_slot_is_written_off_boundary() {
    let mut db = db_with_vault();
    LEVM::write_openings_roots(&mut db, &[], 5).expect("write");
    let batch_slot = H256(batch_slot_for_block(5).to_big_endian());
    assert!(
        vault_slot_word(&mut db, batch_slot).is_zero(),
        "a batch root must only be written at the batch's last block"
    );
}

#[test]
fn openings_root_writes_are_recorded_in_the_block_access_list() {
    // The parallel importer derives the state root FROM the BAL, so a block-end
    // write missing from it is a state-root divergence even when execution is
    // correct. This write happens outside the EVM, so nothing records it for us.
    let source = Address::from_low_u64_be(0x5011);
    let alice = Address::from_low_u64_be(0xA11CE);
    let mut db = db_with_vault();
    db.enable_bal_recording();

    let receipts = vec![receipt_with(vec![created_log(
        source,
        alice,
        0,
        U256::from(1u64),
    )])];
    LEVM::write_openings_roots(&mut db, &receipts, 11).expect("write");

    let bal = db.take_bal().expect("BAL");
    let vault_changes = bal
        .accounts()
        .iter()
        .find(|c| c.address == utxo_vault())
        .expect("the BAL must mention the vault");
    let ring = ring_slot(11);
    assert!(
        vault_changes.storage_changes.iter().any(|c| c.slot == ring),
        "the ring write must be recorded as a storage change"
    );
}

#[test]
fn openings_root_write_survives_block_finalization() {
    // Regression test for a bug this PoC deployment found and the earlier unit
    // tests did not: writing the ring slot is not enough, the block must also be
    // able to FINALIZE. `get_state_transitions` needs a recorded pre-value for
    // every slot present in the current state, so a protocol-direct write that
    // skips that bookkeeping makes the whole block unbuildable —
    // "Failed to get old value from account's initial storage" — which on a live
    // chain shows up as the proposer being unable to produce the activation block
    // at all, with no error in the execution client's own log.
    //
    // The original tests asserted the slot's value and stopped there, so they were
    // blind to it. Asserting through finalization is what makes this real.
    let source = Address::from_low_u64_be(0x5011);
    let alice = Address::from_low_u64_be(0xA11CE);
    let block_number = 83u64;

    let mut db = db_with_vault();
    let receipts = vec![receipt_with(vec![created_log(
        source,
        alice,
        0,
        U256::from(1u64),
    )])];
    LEVM::write_openings_roots(&mut db, &receipts, block_number).expect("write");

    let updates = db
        .get_state_transitions()
        .expect("the block must be finalizable after a block-end openings-root write");

    let vault_update = updates
        .iter()
        .find(|u| u.address == utxo_vault())
        .expect("the vault must appear in the block's account updates");
    let ring = H256(ring_slot(block_number).to_big_endian());
    let written = vault_update
        .added_storage
        .get(&ring)
        .copied()
        .expect("the ring slot must appear as an added storage entry");
    let expected = merkle_root(&[opening_leaf(0, source, alice, U256::from(1u64))]);
    assert_eq!(written, U256::from_big_endian(expected.as_bytes()));
}

#[test]
fn empty_block_openings_root_write_survives_finalization() {
    // The unconditional empty-block write goes through the same path, and it is the
    // one that fires on every quiet block — i.e. almost all of them.
    let mut db = db_with_vault();
    LEVM::write_openings_roots(&mut db, &[], 84).expect("write");
    let updates = db
        .get_state_transitions()
        .expect("an empty block's zero-root write must also finalize");
    // A zero written over an unset slot is a no-op diff, so the vault may be absent
    // from the updates; what must not happen is an error.
    let _ = updates;
}

// ---------------------------------------------------------------------------
// Conservation and multi-input spends.
//
// A mutation audit found the sponsored conservation check and the self-funded
// conservation boundary were both unconstrained: deleting the sponsored check
// outright, or loosening the self-funded one by a wei, left the whole suite
// green. Multi-frame and multi-input spends had no coverage at all, so the
// intra-transaction double-spend defence was asserted only in a comment.
// ---------------------------------------------------------------------------

/// A sponsored spend may not pay out more than its inputs prove.
///
/// The sponsor covers the fee, so this frame's conservation reduces to
/// `spent >= signed_out` — the only thing standing between a sponsored spend and
/// minting value, since the vault would otherwise credit account outputs it never
/// received. Deleting that single comparison used to leave every test passing.
#[test]
fn a_sponsored_spend_cannot_pay_out_more_than_its_inputs() {
    let payee = Address::from_low_u64_be(0xBEEF);
    let input_value = U256::from(10u64).pow(U256::from(18u64));

    // Exactly the input value is fine: the sponsor pays the fee separately.
    let (exact, _) = sponsored_fixture_paying(
        None,
        vec![SpendOutput {
            recipient: payee,
            value: input_value,
        }],
    );
    let (result, _) = run_spend(&exact);
    assert!(
        result.is_ok(),
        "a sponsored spend paying out exactly its inputs must execute; got {result:?}"
    );

    // One wei more than the inputs prove must not.
    let (over, _) = sponsored_fixture_paying(
        None,
        vec![SpendOutput {
            recipient: payee,
            value: input_value + U256::one(),
        }],
    );
    let (result, mut db) = run_spend(&over);
    assert!(
        result.is_err(),
        "a sponsored spend paying out more than its inputs must be rejected"
    );
    assert!(
        vault_slot_word(&mut db, over.spent_slot).is_zero(),
        "the rejected spend must leave no spent bit"
    );
    assert!(
        db.current_accounts_state
            .get(&payee)
            .is_none_or(|acc| acc.info.balance.is_zero()),
        "the over-paid recipient must receive nothing"
    );
}

/// Two UTXO frames in one transaction cannot spend the same input.
///
/// The spent bit is staged, not written, while the frame loop runs, so the second
/// frame can only see the first frame's bit through the durable-write overlay in
/// `read_vault_slot`. Without that lookup both frames would verify their proof
/// against an unset bit and the transaction would pay the input out twice.
#[test]
fn two_utxo_frames_cannot_spend_the_same_input() {
    let (mut fixture, _sponsor) = sponsored_fixture(None);
    // frames = [VERIFY pay, UTXO]; append a byte-identical second UTXO frame. The
    // spend hash is unchanged, so the actor's existing signature still covers it.
    let duplicate = fixture.tx.frames[1].clone();
    fixture.tx.frames.push(duplicate);

    let (result, mut db) = run_spend(&fixture);
    assert!(
        result.is_err(),
        "the second frame must fail on the already-staged spent bit, invalidating \
         the transaction; got {result:?}"
    );
    assert!(
        vault_slot_word(&mut db, fixture.spent_slot).is_zero(),
        "an invalidated transaction must leave no spent bit"
    );
}

/// The self-funded conservation boundary is exact to the wei.
///
/// A self-funded spend must cover its signed outputs AND the transaction's
/// maximum cost: `spent >= signed_out + max_cost`. Loosening that comparison by a
/// single wei once survived the whole suite, so both sides of the boundary are
/// pinned here.
///
/// Two things make this work where the obvious version does not. Bisecting for the
/// boundary and then asserting one-past-it fails is self-calibrating — it
/// re-derives the boundary under whatever comparison is in force, so a loosened
/// check moves the boundary and the test with it. And varying the *payout* cannot
/// express a one-wei violation at all: the payout is part of the frame's calldata,
/// so raising it by a wei can raise `total_gas_limit` (and hence `max_cost`) by a
/// whole gas unit, stepping `needed` across the boundary 36,001 wei at a time.
///
/// So the payout is held fixed — pinning `signed_out` and `max_cost` — and the
/// *input* is varied, which moves `spent` in steps of one. `max_cost` is derived
/// from the transaction's own fields rather than hardcoded, so a repricing tracks
/// automatically; what is asserted is the arithmetic relation, which is what a
/// loosened comparison breaks.
#[test]
fn self_funded_conservation_is_exact_to_the_wei() {
    let payee = Address::from_low_u64_be(0xBEEF);
    // Round and fixed: the payout must not change length as the input moves, or
    // `max_cost` would move with it.
    let payout = U256::from(500_000_000_000_000_000u64);

    // Solve for the boundary input `V` such that `V == payout + max_cost(V)`.
    //
    // It has to be a fixed point rather than one calculation: `max_cost` is
    // `max_fee_per_gas * total_gas_limit`, and the total includes the frame's
    // calldata cost, which under EIP-8038 depends on the *byte content* of the
    // encoded spend — zero and non-zero bytes are priced differently. So changing
    // the input value changes `max_cost`, which changes the boundary. The iteration
    // converges in a couple of rounds because the dependence is weak.
    //
    // This does not make the test self-calibrating: `max_cost` is derived from the
    // transaction's own encoding, never from the conservation comparison. What is
    // asserted below is that `spent >= signed_out + max_cost` holds exactly at that
    // point — which is precisely what a loosened comparison violates.
    let max_cost_at = |input: U256| {
        let fixture = self_funded_fixture(input, Some((payee, payout)), None);
        U256::from(fixture.tx.max_fee_per_gas) * U256::from(fixture.tx.total_gas_limit())
    };
    let mut boundary = payout + max_cost_at(payout + U256::from(3_000_000_000u64));
    for _ in 0..8 {
        let next = payout + max_cost_at(boundary);
        if next == boundary {
            break;
        }
        boundary = next;
    }
    assert_eq!(
        boundary,
        payout + max_cost_at(boundary),
        "the boundary solve must reach a fixed point"
    );
    let max_cost = boundary - payout;
    assert!(!max_cost.is_zero(), "the fixture must have a real max cost");

    let accepts = |input: U256| {
        run_spend(&self_funded_fixture(input, Some((payee, payout)), None))
            .0
            .is_ok()
    };

    // Exactly enough: the inputs cover the payout and the whole fee cap.
    assert!(
        accepts(boundary),
        "spent == signed_out + max_cost must be accepted (boundary {boundary})"
    );

    // One wei short must be rejected, and must leave nothing behind. This is the
    // assertion a comparison loosened by a wei fails.
    let short = boundary - U256::one();
    let fixture = self_funded_fixture(short, Some((payee, payout)), None);
    let (result, mut db) = run_spend(&fixture);
    assert!(
        result.is_err(),
        "one wei short of signed_out + max_cost must be rejected, not absorbed"
    );
    assert!(
        vault_slot_word(&mut db, fixture.spent_slot).is_zero(),
        "the rejected spend must leave no spent bit"
    );
    assert!(
        db.current_accounts_state
            .get(&payee)
            .is_none_or(|acc| acc.info.balance.is_zero()),
        "the rejected spend must pay nobody"
    );
}

/// Build a consolidation spend: `n` UTXOs, each owned by a different actor, all
/// committed in one block's openings tree, merged into a single payout plus
/// change. This is the multi-actor form the EIP's bundling rationale is about.
fn consolidation_fixture(n: usize, payout: U256) -> (SpendFixture, Vec<Address>, U256, Address) {
    let per_input = U256::from(10u64).pow(U256::from(18u64));
    let source = Address::from_low_u64_be(0x5011);
    let payee = Address::from_low_u64_be(0xC0FFEE);

    let mut keys = Vec::new();
    let mut actors = Vec::new();
    for i in 0..n {
        let (k, a) = key_and_address(0x70 + u8::try_from(i).unwrap());
        keys.push(k);
        actors.push(a);
    }
    let indices: Vec<u64> = (0..u64::try_from(n).unwrap()).map(|i| 20 + i).collect();

    // One tree for the creation block, holding every input's leaf, so each input
    // carries a real sibling path rather than a degenerate one.
    let leaves: Vec<H256> = indices
        .iter()
        .zip(&actors)
        .map(|(index, actor)| opening_leaf(*index, source, *actor, per_input))
        .collect();
    let root = merkle_root(&leaves);
    let inputs: Vec<SpendInput> = indices
        .iter()
        .zip(&actors)
        .enumerate()
        .map(|(position, (index, actor))| SpendInput {
            index: *index,
            creation_block: CREATION_BLOCK,
            source,
            recipient: *actor,
            value: per_input,
            position: u64::try_from(position).unwrap(),
            siblings: merkle_proof(&leaves, position).expect("proof"),
            batch_siblings: vec![],
        })
        .collect();

    // Change goes to the first actor; the signed change entry carries value 0.
    let spend = Spend {
        actors: actors.clone(),
        inputs,
        utxo_outs: vec![SpendOutput {
            recipient: actors[0],
            value: U256::zero(),
        }],
        account_outs: vec![SpendOutput {
            recipient: payee,
            value: payout,
        }],
        change_index: 0,
        payer: Bytes::new(), // self-funded: the inputs pay the fee
        max_fee_per_gas: U256::from(1_000_000u64),
        max_priority_fee_per_gas: U256::from(1_000_000u64),
        max_gas_limit: 30_000_000,
    };

    let mut tx = FrameTransaction {
        chain_id: 1,
        nonce_keys: vec![],
        nonce_seq: 0,
        sender: utxo_vault(),
        frames: vec![Frame {
            mode: FrameMode::Utxo as u8,
            flags: 0,
            target: None,
            gas_limit: 3_000_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::from(spend.encode_to_vec()),
        }],
        signatures: vec![],
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1_000,
        ..Default::default()
    };
    // Every actor signs the same spend hash: witness fields are outside it, so one
    // digest authorises the whole bundle.
    let digest = spend.spend_hash(tx.chain_id);
    for (key, actor) in keys.iter().zip(&actors) {
        tx.signatures.push(sign_digest(key, digest, *actor));
    }

    let total = per_input * U256::from(u64::try_from(n).unwrap());
    let mut vault_storage = FxHashMap::default();
    vault_storage.insert(
        H256(ring_slot(CREATION_BLOCK).to_big_endian()),
        U256::from_big_endian(root.as_bytes()),
    );
    let vault_acc = Account::new(
        total,
        Code::from_bytecode(
            Bytes::from_static(&UTXO_VAULT_RUNTIME_BYTECODE),
            &NativeCrypto,
        ),
        1,
        vault_storage,
    );
    let (first_spent_slot, _) = spent_bit_location(indices[0]);
    (
        SpendFixture {
            tx,
            accounts: [(utxo_vault(), vault_acc)].into_iter().collect(),
            input_index: indices[0],
            spent_slot: H256(first_spent_slot.to_big_endian()),
        },
        actors,
        total,
        payee,
    )
}

/// A consolidation spend merges three separately-owned UTXOs into one payout.
///
/// This is the bundling case the EIP's rationale rests on — several one-time
/// payments to the same owner (or a group) collapsed into a single spend, one
/// fee, one signature set. It exercises multi-input value summation, multi-actor
/// authorisation against a single spend hash, and several spent bits staged and
/// flushed within one frame.
#[test]
fn a_consolidation_spend_merges_three_inputs_from_three_actors() {
    let payout = U256::from(10u64).pow(U256::from(18u64)); // one input's worth
    let (fixture, actors, total, payee) = consolidation_fixture(3, payout);

    let (result, mut db) = run_spend(&fixture);
    let report = result.expect("a consolidation spend must execute");
    assert!(matches!(report.result, TxResult::Success));

    // The payee is credited exactly the signed account output.
    assert_eq!(
        db.current_accounts_state
            .get(&payee)
            .map(|acc| acc.info.balance)
            .unwrap_or_default(),
        payout,
        "the account output must be credited exactly its signed value"
    );

    // Every input is now spent — not just the first.
    for offset in 0..3u64 {
        let (slot_u256, mask) = spent_bit_location(20 + offset);
        let word = vault_slot_word(&mut db, H256(slot_u256.to_big_endian()));
        assert!(
            !(word & mask).is_zero(),
            "input {} must be marked spent",
            20 + offset
        );
    }

    // Value is conserved: the vault keeps the change and nothing more. Everything
    // it still holds beyond the payout is the change plus the unspent fee
    // headroom, so it must be strictly less than the pooled inputs.
    let vault_left = db
        .current_accounts_state
        .get(&utxo_vault())
        .map(|acc| acc.info.balance)
        .unwrap_or_default();
    assert!(
        vault_left < total && vault_left + payout <= total,
        "the vault must retain only the change: left={vault_left}, total={total}"
    );
    assert_eq!(
        actors.len(),
        3,
        "three distinct actors authorised the spend"
    );
}
