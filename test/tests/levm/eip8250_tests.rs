//! EIP-8250 keyed nonces, executed by the VM the way a block executes them:
//! stateful validity against every selected key, nonce consumption on the
//! payment-scoped APPROVE, first-use state gas, protocol bookkeeping that warms
//! nothing, and the `TXPARAM` indices the EIP adds.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{
    Account, BlockHeader, Code, FRAME_RECEIPT_STATUS_SUCCESS, Fork, Frame, FrameMode,
    FrameTransaction, Transaction, frame_tx_nonce_manager, keyed_nonce_slot,
};
use ethrex_common::{Address, H256, U256, constants::EMPTY_TRIE_HASH, utils::keccak};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::{EVMConfig, Environment};
use ethrex_levm::errors::{ExecutionReport, TxValidationError, VMError};
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use rustc_hash::FxHashMap;
use std::sync::Arc;

const HARNESS_CHAIN_ID: u64 = 1;
const HARNESS_BASE_FEE: u64 = 1;
const FUNDED_SENDER: Address = Address::repeat_byte(0xAA);
fn sender_balance() -> U256 {
    U256::from(10u64).pow(U256::from(18u64))
}

/// APPROVE(scope=3): sender + payment approval; frame target must be the sender.
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA];
/// SSTORE 1@0; REVERT — a state-writing frame that always reverts.
const SSTORE_THEN_REVERT_CODE: &[u8] =
    &[0x60, 0x01, 0x60, 0x00, 0x55, 0x60, 0x00, 0x60, 0x00, 0xFD];
/// EIP-8037 STATE_BYTES_PER_NEW_ACCOUNT * CPSB: the state gas a value-bearing frame
/// pays to bring a fresh account into existence.
const NEW_ACCOUNT_STATE_GAS: u64 = 120 * 1530;
/// EIP-8250 `KEYED_NONCE_FIRST_USE_STATE_GAS` = EIP-8037 STATE_BYTES_PER_STORAGE_SET * CPSB:
/// the state gas a payment approval pays for each keyed-nonce slot it creates.
const KEYED_NONCE_FIRST_USE_STATE_GAS: u64 = 64 * 1530;
/// NONCE_MANAGER predeploy runtime code: PUSH1 0; PUSH1 0; REVERT.
const NONCE_MANAGER_STUB_CODE: &[u8] = &[0x60, 0x00, 0x60, 0x00, 0xFD];

type SeededAccount = (Address, U256, u64, Bytes);

fn seeded_db(accounts: &[SeededAccount]) -> GeneralizedDatabase {
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, header).unwrap());

    let mut cache: FxHashMap<Address, Account> = FxHashMap::default();
    for (address, balance, nonce, code) in accounts {
        cache.insert(
            *address,
            Account::new(
                *balance,
                Code::from_bytecode(code.clone(), &NativeCrypto),
                *nonce,
                FxHashMap::default(),
            ),
        );
    }
    GeneralizedDatabase::new_with_account_state(Arc::new(store), cache)
}

fn frame_tx_env(tx: &FrameTransaction) -> Environment {
    Environment {
        origin: tx.sender,
        gas_limit: tx.max_gas(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(HARNESS_CHAIN_ID),
        base_fee_per_gas: U256::from(HARNESS_BASE_FEE),
        gas_price: tx.max_fee_per_gas,
        tx_nonce: tx.nonce_seq,
        ..Default::default()
    }
}

fn frame_tx_with_keys(frames: Vec<Frame>, nonce_keys: Vec<U256>) -> FrameTransaction {
    FrameTransaction {
        chain_id: HARNESS_CHAIN_ID,
        nonce_keys,
        nonce_seq: 0,
        sender: FUNDED_SENDER,
        frames,
        signatures: Vec::new(),
        max_priority_fee_per_gas: U256::from(1),
        max_fee_per_gas: U256::from(HARNESS_BASE_FEE + 1_000),
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: Vec::new(),
        inner_hash: Default::default(),
        cached_canonical: Default::default(),
    }
}

fn frame(mode: FrameMode, flags: u8, target: Address, gas_limit: u64, data: &[u8]) -> Frame {
    Frame {
        mode: u8::from(mode),
        flags,
        target: Some(target),
        gas_limit,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::from(data.to_vec()),
    }
}

fn run_frame_tx(
    accounts: &[SeededAccount],
    tx: FrameTransaction,
) -> (Result<ExecutionReport, VMError>, GeneralizedDatabase) {
    let mut db = seeded_db(accounts);
    let env = frame_tx_env(&tx);
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
        .expect("VM::new should succeed for a frame tx");
        vm.execute()
    };
    (result, db)
}

fn nonce_of(db: &GeneralizedDatabase, addr: Address) -> u64 {
    db.current_accounts_state
        .get(&addr)
        .map(|account| account.info.nonce)
        .unwrap_or_default()
}

fn storage_slot(db: &GeneralizedDatabase, addr: Address, key: H256) -> U256 {
    db.current_accounts_state
        .get(&addr)
        .and_then(|account| account.storage.get(&key).copied())
        .unwrap_or_default()
}

/// NONCE_MANAGER slot for `(sender, key)`: keccak256(pad32(sender) || be32(key)).
fn keyed_slot(sender: Address, key: U256) -> H256 {
    let mut preimage = [0u8; 64];
    preimage[12..32].copy_from_slice(sender.as_bytes());
    preimage[32..64].copy_from_slice(&key.to_big_endian());
    let slot = H256(ethrex_crypto::keccak::keccak_hash(preimage));
    assert_eq!(slot, keyed_nonce_slot(sender, key));
    slot
}

fn nonce_manager_account() -> SeededAccount {
    (
        frame_tx_nonce_manager(),
        U256::zero(),
        1,
        Bytes::from(NONCE_MANAGER_STUB_CODE.to_vec()),
    )
}

// ==================== consumption survives a LATER batch revert ====================

#[test]
fn key0_consumption_survives_a_later_batch_revert() {
    // Legit shape: frame[0] VERIFY(scope=3, NO batch flag) grants payment and
    // consumes the key-0 nonce; frames[1..] are a SENDER atomic batch that
    // reverts. The payment frame is not in the batch, so its consumption is
    // outside the batch's revert scope and survives.
    let reverter = Address::from_low_u64_be(0x82_50_21);
    let accounts = [
        (
            FUNDED_SENDER,
            sender_balance(),
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (
            reverter,
            U256::zero(),
            0,
            Bytes::from(SSTORE_THEN_REVERT_CODE.to_vec()),
        ),
    ];
    let tx = frame_tx_with_keys(
        vec![
            Frame {
                state_gas_limit: KEYED_NONCE_FIRST_USE_STATE_GAS,
                ..frame(FrameMode::Verify, 0x03, FUNDED_SENDER, 100_000, &[])
            },
            frame(FrameMode::Sender, 0x04, reverter, 100_000, &[]),
            frame(FrameMode::Sender, 0x00, reverter, 100_000, &[]),
        ],
        vec![U256::zero()],
    );
    let (result, db) = run_frame_tx(&accounts, tx);
    let report = result.expect("payment is granted outside the batch, so the tx is valid");
    assert_eq!(report.payer_address, Some(FUNDED_SENDER));
    assert_eq!(
        nonce_of(&db, FUNDED_SENDER),
        1,
        "key-0 consumption from the non-batch payment frame survives the later batch revert",
    );
    assert!(
        storage_slot(&db, reverter, H256::zero()).is_zero(),
        "the reverted in-batch SSTORE must not survive",
    );
}

#[test]
fn keyed_nonce_consumption_survives_a_later_batch_revert() {
    // Same as above but with a non-zero nonce key, so consumption lands in
    // NONCE_MANAGER storage rather than the account nonce.
    let reverter = Address::from_low_u64_be(0x82_50_31);
    let accounts = [
        (
            FUNDED_SENDER,
            sender_balance(),
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (
            reverter,
            U256::zero(),
            0,
            Bytes::from(SSTORE_THEN_REVERT_CODE.to_vec()),
        ),
        nonce_manager_account(),
    ];
    let tx = frame_tx_with_keys(
        vec![
            Frame {
                state_gas_limit: KEYED_NONCE_FIRST_USE_STATE_GAS,
                ..frame(FrameMode::Verify, 0x03, FUNDED_SENDER, 100_000, &[])
            },
            frame(FrameMode::Sender, 0x04, reverter, 100_000, &[]),
            frame(FrameMode::Sender, 0x00, reverter, 100_000, &[]),
        ],
        vec![U256::one()],
    );
    let (result, db) = run_frame_tx(&accounts, tx);
    let report = result.expect("payment granted outside the batch keeps the tx valid");
    assert_eq!(report.payer_address, Some(FUNDED_SENDER));
    assert_eq!(
        storage_slot(
            &db,
            frame_tx_nonce_manager(),
            keyed_slot(FUNDED_SENDER, U256::one())
        ),
        U256::one(),
        "keyed-nonce consumption survives the later batch revert",
    );
    assert_eq!(
        nonce_of(&db, FUNDED_SENDER),
        0,
        "a non-zero key must not touch the sender's linear account nonce",
    );
}

// ==================== Keyed nonces are protocol bookkeeping ====================
//
// EIP-8250: "Keyed-nonce reads and writes performed by stateful validity and
// `consume_nonce_set` are protocol bookkeeping: they do NOT add `NONCE_MANAGER`
// or its slots to EIP-2929 `accessed_addresses` or `accessed_storage_keys`, are
// NOT charged under EIP-2200 `SSTORE` pricing, and do NOT warm the address or
// slot for later user-level access."

/// A probe account whose code does `PUSH20 addr; BALANCE; POP; STOP`.
const PROBE: Address = Address::repeat_byte(0xB2);
/// Never touched by the protocol: the cold control.
const COLD_CONTROL: Address = Address::repeat_byte(0xC2);

fn balance_probe_code(addr: Address) -> Bytes {
    let mut code = vec![0x73];
    code.extend_from_slice(addr.as_bytes());
    code.extend_from_slice(&[0x31, 0x50, 0x00]);
    Bytes::from(code)
}

/// Run a frame tx that consumes `nonce_keys` and then probes `probed`'s
/// warm/cold status from a DEFAULT frame, returning the execution report. The
/// VERIFY frame carries the state budget for two fresh keys, which is what the
/// probes below consume at most.
fn keyed_nonce_probe(probed: Address, nonce_keys: Vec<U256>) -> ExecutionReport {
    let tx = frame_tx_with_keys(
        vec![
            Frame {
                state_gas_limit: 2 * KEYED_NONCE_FIRST_USE_STATE_GAS,
                ..frame(FrameMode::Verify, 0x03, FUNDED_SENDER, 100_000, &[])
            },
            frame(FrameMode::Default, 0, PROBE, 100_000, &[]),
        ],
        nonce_keys,
    );
    let (result, _db) = run_frame_tx(
        &[
            (
                FUNDED_SENDER,
                sender_balance(),
                0,
                Bytes::from(APPROVE_BOTH_CODE.to_vec()),
            ),
            (PROBE, U256::zero(), 0, balance_probe_code(probed)),
            (COLD_CONTROL, U256::from(1u64), 0, Bytes::new()),
            (
                frame_tx_nonce_manager(),
                U256::zero(),
                1,
                Bytes::from(NONCE_MANAGER_STUB_CODE.to_vec()),
            ),
        ],
        tx,
    );
    result.expect("the keyed-nonce probe tx must execute")
}

#[test]
fn consuming_a_keyed_nonce_does_not_warm_the_nonce_manager() {
    let nonce_manager = frame_tx_nonce_manager();
    let keys = vec![U256::one()];

    let manager_gas = keyed_nonce_probe(nonce_manager, keys.clone()).gas_used;
    let control_gas = keyed_nonce_probe(COLD_CONTROL, keys).gas_used;

    assert_eq!(
        manager_gas, control_gas,
        "consuming a keyed nonce must not warm NONCE_MANAGER for later \
         user-level access: probing it cost {manager_gas} against {control_gas} \
         for a never-touched account"
    );
}

#[test]
fn a_keyed_nonce_first_use_is_priced_as_state_gas() {
    // EIP-8250 §Nonce consumption: the only keyed-nonce charge is one storage set of
    // state gas per newly-occupied key, drawn from the approving frame's
    // `limits.state`. It is not execution gas, not an EIP-2200 SSTORE charge, and
    // not a cold-slot access on top.
    let legacy_only = keyed_nonce_probe(COLD_CONTROL, vec![U256::zero()]);
    let one_key = keyed_nonce_probe(COLD_CONTROL, vec![U256::one()]);
    let two_keys = keyed_nonce_probe(COLD_CONTROL, vec![U256::one(), U256::from(2u64)]);

    // The state dimension: exactly one first-use charge per fresh key, and none
    // for key 0, which is the account nonce and occupies no NONCE_MANAGER slot.
    assert_eq!(
        one_key.state_gas_used - legacy_only.state_gas_used,
        KEYED_NONCE_FIRST_USE_STATE_GAS,
        "one fresh key must cost exactly one KEYED_NONCE_FIRST_USE_STATE_GAS of state gas"
    );
    assert_eq!(
        two_keys.state_gas_used - one_key.state_gas_used,
        KEYED_NONCE_FIRST_USE_STATE_GAS,
        "a second fresh key must cost exactly one more KEYED_NONCE_FIRST_USE_STATE_GAS"
    );

    // The execution dimension: an extra key only lengthens the signed envelope, so
    // the delta is a few gas of ordinary transaction data cost. A cold slot access
    // or an EIP-2200 charge would add thousands, so bounding the excess below
    // `ENVELOPE_SLACK` is what makes this an assertion about pricing.
    const ENVELOPE_SLACK: u64 = 100;
    let execution = |report: &ExecutionReport| report.gas_used - report.state_gas_used;
    let keyed_delta = execution(&one_key) - execution(&legacy_only);
    assert!(
        keyed_delta < ENVELOPE_SLACK,
        "consuming a fresh keyed nonce must cost no execution gas beyond envelope data, \
         but cost {keyed_delta} more"
    );
    let second_key_delta = execution(&two_keys) - execution(&one_key);
    assert!(
        second_key_delta < ENVELOPE_SLACK,
        "a second fresh key must cost no execution gas beyond envelope data, but cost \
         {second_key_delta} more -- a storage charge is layered on top"
    );
}

// ==================== Contract sender on a keyed nonce ====================

/// A contract sender whose runtime is nothing but `APPROVE(3)`, a VERIFY frame on
/// itself, and a SENDER frame paying a fresh address, all on a keyed nonce rather
/// than the legacy one. The VERIFY frame declares exactly the state budget EIP-8250
/// charges for creating the key's NONCE_MANAGER slot.
#[test]
fn a_contract_sender_can_approve_on_a_first_use_keyed_nonce() {
    let contract = Address::repeat_byte(0xC5);
    let recipient = Address::from_low_u64_be(0xF00D);
    let mut tx = frame_tx_with_keys(
        vec![
            Frame {
                mode: u8::from(FrameMode::Verify),
                flags: 0x03,
                target: Some(contract),
                gas_limit: 80_000,
                state_gas_limit: KEYED_NONCE_FIRST_USE_STATE_GAS,
                value: U256::zero(),
                data: Bytes::new(),
            },
            Frame {
                mode: u8::from(FrameMode::Sender),
                flags: 0,
                target: Some(recipient),
                gas_limit: 30_000,
                state_gas_limit: NEW_ACCOUNT_STATE_GAS,
                value: U256::from(100u64),
                data: Bytes::new(),
            },
        ],
        vec![U256::from(0x9999_0000u64)],
    );
    tx.sender = contract;

    let accounts = [(
        contract,
        U256::from(10u64).pow(U256::from(18u64)), // 1 ETH, as the probe funds it
        0,
        Bytes::from(APPROVE_BOTH_CODE.to_vec()),
    )];
    let (result, db) = run_frame_tx(&accounts, tx);
    let report =
        result.expect("a contract sender approving on a first-use keyed nonce must be a valid tx");
    let frame_results = report.frame_results.expect("frame results present");
    assert_eq!(
        frame_results[1].0, FRAME_RECEIPT_STATUS_SUCCESS,
        "the SENDER frame must deliver its value; got {frame_results:?}"
    );
    assert_eq!(
        db.current_accounts_state
            .get(&recipient)
            .map(|a| a.info.balance)
            .unwrap_or_default(),
        U256::from(100u64),
        "the recipient must actually be funded"
    );
}

/// Two frame transactions from one contract sender on disjoint keys, run in
/// sequence against a single database the way a block runs them, so the second
/// sees everything the first committed. Both must execute.
#[test]
fn two_keyed_transactions_from_one_contract_sender_both_execute() {
    let contract = Address::repeat_byte(0xC5);
    let mut db = seeded_db(&[(
        contract,
        U256::from(10u64).pow(U256::from(18u64)),
        0,
        Bytes::from(APPROVE_BOTH_CODE.to_vec()),
    )]);

    for index in 0u64..2 {
        let recipient = Address::from_low_u64_be(0xBEEF_0000 + index);
        let mut tx = frame_tx_with_keys(
            vec![
                Frame {
                    mode: u8::from(FrameMode::Verify),
                    flags: 0x03,
                    target: Some(contract),
                    gas_limit: 80_000,
                    state_gas_limit: KEYED_NONCE_FIRST_USE_STATE_GAS,
                    value: U256::zero(),
                    data: Bytes::new(),
                },
                Frame {
                    mode: u8::from(FrameMode::Sender),
                    flags: 0,
                    target: Some(recipient),
                    gas_limit: 30_000,
                    state_gas_limit: NEW_ACCOUNT_STATE_GAS,
                    value: U256::from(100u64),
                    data: Bytes::new(),
                },
            ],
            vec![U256::from(0x8250_0000u64 + index)],
        );
        tx.sender = contract;

        let env = frame_tx_env(&tx);
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
            .expect("VM::new should succeed for a frame tx");
            vm.execute()
        };
        let report = result.unwrap_or_else(|e| {
            panic!("transaction {index} on key {index} must be valid, got {e:?}")
        });
        let frames = report.frame_results.expect("frame results present");
        assert_eq!(
            frames[1].0, FRAME_RECEIPT_STATUS_SUCCESS,
            "transaction {index}'s SENDER frame must deliver its value; got {frames:?}"
        );
        assert_eq!(
            db.current_accounts_state
                .get(&recipient)
                .map(|a| a.info.balance)
                .unwrap_or_default(),
            U256::from(100u64),
            "transaction {index}'s recipient must be funded"
        );
    }
}

/// The same transaction with a VERIFY frame that declares no state budget: the
/// first-use charge cannot be covered, the frame halts with no approval effect,
/// and a halted VERIFY frame invalidates the transaction.
#[test]
fn a_first_use_keyed_nonce_halts_the_frame_without_its_state_budget() {
    let contract = Address::repeat_byte(0xC5);
    let mut tx = frame_tx_with_keys(
        vec![
            Frame {
                mode: u8::from(FrameMode::Verify),
                flags: 0x03,
                target: Some(contract),
                gas_limit: 80_000,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
            },
            Frame {
                mode: u8::from(FrameMode::Sender),
                flags: 0,
                target: Some(Address::from_low_u64_be(0xF00D)),
                gas_limit: 30_000,
                state_gas_limit: NEW_ACCOUNT_STATE_GAS,
                value: U256::from(100u64),
                data: Bytes::new(),
            },
        ],
        vec![U256::from(0x9999_0000u64)],
    );
    tx.sender = contract;

    let accounts = [(
        contract,
        U256::from(10u64).pow(U256::from(18u64)),
        0,
        Bytes::from(APPROVE_BOTH_CODE.to_vec()),
    )];
    let (result, db) = run_frame_tx(&accounts, tx);
    let err = result.expect_err(
        "a VERIFY frame that cannot pay the keyed-nonce first-use state gas must halt \
         and invalidate the transaction",
    );
    assert!(
        matches!(
            err,
            VMError::TxValidation(TxValidationError::InvalidFrameTransaction)
        ),
        "the rejection must be the failed VERIFY frame, got {err:?}"
    );
    // No approval effect applied: the key's slot was never written.
    let slot = keyed_slot(contract, U256::from(0x9999_0000u64));
    assert!(
        storage_slot(&db, frame_tx_nonce_manager(), slot).is_zero(),
        "a halted approval must leave the keyed-nonce slot unwritten"
    );
}

// ==================== Key 0 from a sender that does not exist ====================

/// EIP-8250 §Nonce consumption, step 1, on the legacy key: when `tx.sender` does not
/// exist under EIP-8037's existence rule, the approving frame pays account creation
/// state gas immediately before the nonce increment creates the account. The sender
/// here is a never-seen EOA with no balance, so a paymaster's `APPROVE(APPROVE_PAYMENT)`
/// is what consumes the nonce, and it is that frame's `limits.state` that pays.
mod key_zero_from_a_fresh_sender {
    use super::*;
    use ethrex_common::types::{FRAME_SIG_SCHEME_SECP256K1, FrameSignature};
    use k256::ecdsa::SigningKey;

    /// APPROVE(scope=1): payment approval from a paymaster's own code.
    const APPROVE_PAYMENT_CODE: &[u8] = &[0x60, 0x01, 0x60, 0x00, 0x60, 0x00, 0xAA];
    const PAYMASTER: Address = Address::repeat_byte(0xFA);

    fn key_and_address(seed: u8) -> (SigningKey, Address) {
        let signing_key = SigningKey::from_bytes(&[seed; 32].into()).unwrap();
        let uncompressed = signing_key.verifying_key().to_encoded_point(false);
        let pub_hash = ethrex_crypto::keccak::keccak_hash(&uncompressed.as_bytes()[1..]);
        (signing_key, Address::from_slice(&pub_hash[12..]))
    }

    fn sign(key: &SigningKey, sig_hash: H256, signer: Address) -> FrameSignature {
        let (raw_sig, recovery_id) = key.sign_prehash_recoverable(sig_hash.as_bytes()).unwrap();
        let mut bytes = vec![0u8; 65];
        bytes[0] = recovery_id.to_byte();
        bytes[1..].copy_from_slice(&raw_sig.to_bytes());
        FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(signer),
            msg: Bytes::new(),
            signature: Bytes::from(bytes),
        }
    }

    /// A signed `[VERIFY(exec) on the sender's default code, VERIFY(pay) on the
    /// paymaster]` transaction from a sender that exists nowhere, with `pay_state`
    /// as the paymaster frame's state budget.
    fn sponsored_tx_from_nowhere(pay_state: u64) -> (FrameTransaction, Address) {
        let (key, sender) = key_and_address(0x5A);
        let mut tx = frame_tx_with_keys(
            vec![
                frame(FrameMode::Verify, 0x02, sender, 100_000, &[]),
                Frame {
                    state_gas_limit: pay_state,
                    ..frame(FrameMode::Verify, 0x01, PAYMASTER, 100_000, &[])
                },
            ],
            vec![U256::zero()],
        );
        tx.sender = sender;
        tx.signatures = vec![FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(sender),
            msg: Bytes::new(),
            signature: Bytes::from(vec![0u8; 65]),
        }];
        let sig_hash = tx.compute_sig_hash();
        tx.signatures[0] = sign(&key, sig_hash, sender);
        tx.inner_hash = Default::default();
        tx.cached_canonical = Default::default();
        (tx, sender)
    }

    fn paymaster() -> SeededAccount {
        (
            PAYMASTER,
            U256::from(10u64).pow(U256::from(18u64)),
            0,
            Bytes::from(APPROVE_PAYMENT_CODE.to_vec()),
        )
    }

    #[test]
    fn the_approving_frame_pays_account_creation() {
        let (tx, sender) = sponsored_tx_from_nowhere(NEW_ACCOUNT_STATE_GAS);
        let (result, db) = run_frame_tx(&[paymaster()], tx);
        let report = result.expect("a sponsored transaction from a fresh sender must be valid");
        let frames = report.frame_results.expect("frame results present");
        assert_eq!(
            frames[1].2, NEW_ACCOUNT_STATE_GAS,
            "the paymaster frame must be attributed the sender's account creation, got {frames:?}"
        );
        assert_eq!(
            frames[0].2, 0,
            "the sender's own VERIFY frame creates nothing"
        );
        assert_eq!(
            nonce_of(&db, sender),
            1,
            "the increment created the account at nonce 1"
        );
    }

    #[test]
    fn without_the_state_budget_the_approval_halts() {
        let (tx, sender) = sponsored_tx_from_nowhere(NEW_ACCOUNT_STATE_GAS - 1);
        let (result, db) = run_frame_tx(&[paymaster()], tx);
        result.expect_err(
            "a paymaster frame one gas short of the account creation charge must halt and \
             invalidate the transaction",
        );
        assert_eq!(
            nonce_of(&db, sender),
            0,
            "no approval effect may survive the halt"
        );
    }
}

// ==================== Stateful validity against every selected key ====================

fn contract_sender() -> SeededAccount {
    (
        FUNDED_SENDER,
        sender_balance(),
        0,
        Bytes::from(APPROVE_BOTH_CODE.to_vec()),
    )
}

/// A self-approving transaction on `nonce_keys` at `nonce_seq`, with state budget
/// for every key's first use.
fn keyed_tx(nonce_keys: Vec<U256>, nonce_seq: u64) -> FrameTransaction {
    let state = KEYED_NONCE_FIRST_USE_STATE_GAS * nonce_keys.len() as u64;
    let mut tx = frame_tx_with_keys(
        vec![Frame {
            state_gas_limit: state,
            ..frame(FrameMode::Verify, 0x03, FUNDED_SENDER, 100_000, &[])
        }],
        nonce_keys,
    );
    tx.nonce_seq = nonce_seq;
    tx
}

fn execute_on(
    db: &mut GeneralizedDatabase,
    tx: FrameTransaction,
) -> Result<ExecutionReport, VMError> {
    let env = frame_tx_env(&tx);
    let transaction = Transaction::FrameTransaction(tx);
    let mut vm = VM::new(
        env,
        db,
        &transaction,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("VM::new should succeed for a frame tx");
    vm.execute()
}

fn assert_nonce_mismatch(result: &Result<ExecutionReport, VMError>, expected: u64, actual: u64) {
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(TxValidationError::NonceMismatch { expected: e, actual: a }))
                if *e == expected && *a == actual
        ),
        "expected a nonce mismatch (expected {expected}, actual {actual}), got {result:?}"
    );
}

#[test]
fn every_selected_key_must_be_at_the_transaction_sequence() {
    let mut db = seeded_db(&[contract_sender()]);
    let nonce_manager = frame_tx_nonce_manager();

    // A fresh key reads as sequence 0, so only `nonce_seq = 0` is valid on it.
    assert_nonce_mismatch(&execute_on(&mut db, keyed_tx(vec![U256::one()], 1)), 0, 1);

    // Consuming [1, 2] at 0 writes 1 to both slots and leaves the account nonce alone.
    let first = execute_on(&mut db, keyed_tx(vec![U256::one(), U256::from(2)], 0))
        .expect("first use of two keys at sequence 0 is valid");
    assert_eq!(
        first.state_gas_used,
        2 * KEYED_NONCE_FIRST_USE_STATE_GAS,
        "two fresh keys pay two first-use charges"
    );
    for key in [U256::one(), U256::from(2)] {
        assert_eq!(
            storage_slot(&db, nonce_manager, keyed_slot(FUNDED_SENDER, key)),
            U256::one()
        );
    }
    assert_eq!(nonce_of(&db, FUNDED_SENDER), 0);

    // An overlapping set is valid only if every key is at its sequence: key 2 is at
    // 1 and key 3 is at 0, so neither sequence satisfies [2, 3].
    assert_nonce_mismatch(
        &execute_on(&mut db, keyed_tx(vec![U256::from(2), U256::from(3)], 0)),
        1,
        0,
    );
    assert_nonce_mismatch(
        &execute_on(&mut db, keyed_tx(vec![U256::from(2), U256::from(3)], 1)),
        0,
        1,
    );

    // Reusing [1, 2] at 1 is valid, creates no slot, and so pays no state gas.
    let reuse = execute_on(&mut db, keyed_tx(vec![U256::one(), U256::from(2)], 1))
        .expect("the next sequence on both keys is valid");
    assert_eq!(
        reuse.state_gas_used, 0,
        "a reused key pays no first-use charge"
    );
    for key in [U256::one(), U256::from(2)] {
        assert_eq!(
            storage_slot(&db, nonce_manager, keyed_slot(FUNDED_SENDER, key)),
            U256::from(2)
        );
    }

    // The legacy domain is independent: [0] still runs at the account nonce 0.
    execute_on(&mut db, keyed_tx(vec![U256::zero()], 0))
        .expect("the legacy key is unaffected by keyed consumption");
    assert_eq!(nonce_of(&db, FUNDED_SENDER), 1);
}

#[test]
fn the_maximum_sequence_is_never_valid() {
    let mut db = seeded_db(&[contract_sender()]);
    let result = execute_on(&mut db, keyed_tx(vec![U256::one()], u64::MAX));
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(TxValidationError::NonceIsMax))
        ),
        "nonce_seq = 2**64 - 1 must be invalid, got {result:?}"
    );
}

// ==================== TXPARAM indices ====================

/// A probe whose code stores `TXPARAM(i)` at slot `i` for every index EIP-8250
/// adds or redefines.
const PROBED_INDICES: [u8; 5] = [0x01, 0x0D, 0x0E, 0x0F, 0x10];

fn txparam_probe_code() -> Bytes {
    let mut code = Vec::new();
    for index in PROBED_INDICES {
        // PUSH1 index; TXPARAM; PUSH1 index; SSTORE
        code.extend_from_slice(&[0x60, index, 0xB0, 0x60, index, 0x55]);
    }
    code.push(0x00);
    Bytes::from(code)
}

fn probe_txparams(sender_nonce: u64, nonce_keys: Vec<U256>, nonce_seq: u64) -> [U256; 5] {
    let probe = Address::repeat_byte(0xB7);
    let accounts = [
        (
            FUNDED_SENDER,
            sender_balance(),
            sender_nonce,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (probe, U256::zero(), 1, txparam_probe_code()),
    ];
    let mut tx = frame_tx_with_keys(
        vec![
            Frame {
                state_gas_limit: KEYED_NONCE_FIRST_USE_STATE_GAS * nonce_keys.len() as u64,
                ..frame(FrameMode::Verify, 0x03, FUNDED_SENDER, 100_000, &[])
            },
            Frame {
                state_gas_limit: 5 * KEYED_NONCE_FIRST_USE_STATE_GAS,
                ..frame(FrameMode::Default, 0, probe, 300_000, &[])
            },
        ],
        nonce_keys,
    );
    tx.nonce_seq = nonce_seq;
    let (result, db) = run_frame_tx(&accounts, tx);
    let report = result.expect("the probe transaction is valid");
    let frames = report.frame_results.expect("frame results present");
    assert_eq!(frames[1].0, FRAME_RECEIPT_STATUS_SUCCESS, "{frames:?}");
    PROBED_INDICES.map(|index| storage_slot(&db, probe, H256::from_low_u64_be(index.into())))
}

#[test]
fn txparam_exposes_the_nonce_fields() {
    let keys = vec![U256::one(), U256::from(0x8250)];
    let [seq, legacy, count, keys_hash, key_0] = probe_txparams(7, keys, 0);

    let mut preimage = Vec::new();
    for word in [U256::from(2), U256::one(), U256::from(0x8250)] {
        preimage.extend_from_slice(&word.to_big_endian());
    }
    assert_eq!(seq, U256::zero(), "0x01 is nonce_seq");
    assert_eq!(legacy, U256::from(7), "0x0D is the pre-state account nonce");
    assert_eq!(count, U256::from(2), "0x0E is len(nonce_keys)");
    assert_eq!(
        keys_hash,
        U256::from_big_endian(keccak(&preimage).as_bytes()),
        "0x0F is keccak256(be32(len) || be32(key)...)"
    );
    assert_eq!(key_0, U256::one(), "0x10 is nonce_keys[0]");
}

#[test]
fn txparam_legacy_nonce_is_not_updated_by_payment_approval() {
    // On [0] the payment approval in frame 0 increments the account nonce to 8,
    // but 0x0D stays the value observed before any frame executed.
    let [seq, legacy, count, _, key_0] = probe_txparams(7, vec![U256::zero()], 7);
    assert_eq!(seq, U256::from(7));
    assert_eq!(legacy, U256::from(7));
    assert_eq!(count, U256::one());
    assert_eq!(key_0, U256::zero());
}
