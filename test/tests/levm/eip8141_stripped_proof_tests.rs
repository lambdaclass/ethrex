//! EIP-8141 §Aggregation: stripped proof frames.
//!
//! From LStar a block may carry a frame transaction without the proofs of its
//! `LEANSPHINCS_VERIFIER` proof frames: the frame keeps its claim and the length
//! of its proof, the form the signature hash gives it, and the block's aggregate
//! proof stands for the proof that left. These tests cover what that changes for
//! one transaction, its encoding and its hash, and what it does not: its
//! signature hash, its gas and everything its execution observes.

use std::sync::Arc;

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, H160, H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{
        Account, BlockHeader, Code, FRAME_TX_LEANSPHINCS_VERIFIER, FRAME_TX_SECP256K1_VERIFIER,
        Fork, Frame, FrameMode, FrameTransaction, LEANSPHINCS_VERIFIER_GAS,
        LEANSPHINCS_VERIFIER_PROOF_LENGTH, PROOF_COMMIT_FLAG, PROOF_READ_FLAG, Transaction,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    db::gen_db::GeneralizedDatabase,
    environment::{EVMConfig, Environment},
    errors::{ExecutionReport, TxValidationError, VMError},
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use rustc_hash::FxHashMap;

pub(crate) const RECIPIENT: Address = H160([0x86; 20]);
pub(crate) const GWEI: u64 = 1_000_000_000;
/// Length in bytes of a leanSPHINCS signature.
pub(crate) const SIGNATURE_SIZE: usize = 7740;
/// What a proof frame pays to reach a verifier precompile, which is warm.
const WARM_ACCESS: u64 = 100;
const CHAIN_ID: u64 = 1;
const BASE_FEE: u64 = 1;

/// A leanSPHINCS key pair and the account it controls.
pub(crate) struct Key {
    pub(crate) secret: [u8; 32],
    pub(crate) public: H256,
    pub(crate) account: Address,
}

pub(crate) fn key(seed: u8) -> Key {
    let secret = [seed; 32];
    Key {
        secret,
        public: H256(lean_sphincs::key_gen(&secret).to_bytes()),
        account: H160([seed; 20]),
    }
}

/// The key's signature of `message`.
pub(crate) fn sign(key: &Key, message: H256) -> Vec<u8> {
    lean_sphincs::sign(&key.secret, message.as_fixed_bytes())
        .to_bytes()
        .to_vec()
}

/// Validation code of an account controlled by the leanSPHINCS key `public_key`:
/// approve execution and payment if frame 0 claims that key, through the
/// leanSPHINCS verifier, for the canonical signature hash. The protocol settled
/// the claim before this code runs, so reading it is enough.
pub(crate) fn account_code(public_key: H256) -> Vec<u8> {
    let mut code = vec![
        0x60, 0x01, 0x60, 0x00, 0xB4, // PROOFPARAM(0x01, 0): verifier
        0x60, 0x14, 0x14, // == LEANSPHINCS_VERIFIER
        0x60, 0x00, 0x60, 0x00, 0xB4, // PROOFPARAM(0x00, 0): key
        0x7F, // PUSH32 public_key
    ];
    code.extend_from_slice(public_key.as_bytes());
    code.extend_from_slice(&[
        0x14, 0x16, // EQ; AND
        0x60, 0x02, 0x60, 0x00, 0xB4, // PROOFPARAM(0x02, 0): msg
        0x15, 0x16, // ISZERO; AND
    ]);
    let approve = u8::try_from(code.len() + 8).expect("the code is shorter than 256 bytes");
    code.extend_from_slice(&[
        0x60, approve, 0x57, // PUSH1 approve; JUMPI
        0x60, 0x00, 0x60, 0x00, 0xFD, // REVERT(0, 0)
        0x5B, // JUMPDEST (approve)
        0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA, // APPROVE(0, 0, 3)
    ]);
    code
}

/// A proof frame for the leanSPHINCS verifier claiming that `key` authorizes
/// `msg`, a zero `msg` standing for the canonical signature hash, with a
/// placeholder of a signature's length for its proof.
pub(crate) fn leansphincs_proof_frame(key: H256, msg: H256, flags: u8) -> Frame {
    let mut data = key.as_bytes().to_vec();
    data.extend_from_slice(msg.as_bytes());
    data.extend_from_slice(&[0u8; SIGNATURE_SIZE]);
    Frame {
        mode: u8::from(FrameMode::Proof),
        flags,
        target: Some(FRAME_TX_LEANSPHINCS_VERIFIER),
        // The verifier's cost plus the access of a warm precompile.
        gas_limit: LEANSPHINCS_VERIFIER_GAS + WARM_ACCESS,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::from(data),
    }
}

/// Replaces the proof of proof frame `index` by `signer`'s signature of the
/// message the frame claims.
pub(crate) fn sign_proof_frame(tx: &mut FrameTransaction, index: usize, signer: &Key) {
    let claimed = tx.frames[index].proof_msg();
    let message = if claimed.is_zero() {
        tx.compute_sig_hash()
    } else {
        claimed
    };
    let mut data = tx.frames[index].proof_claim().to_vec();
    data.extend_from_slice(&sign(signer, message));
    tx.frames[index].data = Bytes::from(data);
    tx.inner_hash = Default::default();
    tx.cached_canonical = Default::default();
}

/// The frames that follow the proof frames of the transactions of these tests:
/// the account's validation frame and a call to `RECIPIENT`.
pub(crate) fn verify_and_sender_frames(account: Address) -> [Frame; 2] {
    [
        Frame {
            mode: u8::from(FrameMode::Verify),
            flags: 0x03,
            target: Some(account),
            gas_limit: 50_000,
            state_gas_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0,
            target: Some(RECIPIENT),
            gas_limit: 50_000,
            state_gas_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ]
}

/// A frame transaction from the key's account with `proof_frames` ahead of its
/// validation frame and its call, not signed yet.
pub(crate) fn transaction_with_proof_frames(
    chain_id: u64,
    key: &Key,
    nonce: u64,
    proof_frames: Vec<Frame>,
) -> FrameTransaction {
    let mut frames = proof_frames;
    frames.extend(verify_and_sender_frames(key.account));
    let mut tx = FrameTransaction {
        chain_id,
        nonce_seq: nonce,
        sender: key.account,
        frames,
        max_priority_fee_per_gas: U256::from(GWEI),
        ..Default::default()
    };
    tx.max_fee = U256::from(10 * GWEI) * U256::from(tx.max_gas());
    tx
}

/// `[PROOF, VERIFY, SENDER]` from the key's account, signed by the key: a proof
/// frame claiming the account's key for the canonical signature hash, with
/// `proof_flags`, the account's validation frame, and a call to `RECIPIENT`.
pub(crate) fn proof_transaction(
    chain_id: u64,
    key: &Key,
    nonce: u64,
    proof_flags: u8,
) -> FrameTransaction {
    let mut tx = transaction_with_proof_frames(
        chain_id,
        key,
        nonce,
        vec![leansphincs_proof_frame(
            key.public,
            H256::zero(),
            proof_flags,
        )],
    );
    // The proof is elided from the signature hash, so it is signed last.
    sign_proof_frame(&mut tx, 0, key);
    tx
}

/// The transaction with every proof the protocol may strip stripped, as a block
/// from LStar carries it.
pub(crate) fn stripped(tx: &FrameTransaction) -> FrameTransaction {
    let mut stripped = tx.clone();
    stripped.strip_proofs();
    stripped
}

fn tx_hash(tx: &FrameTransaction) -> H256 {
    Transaction::FrameTransaction(tx.clone()).hash(&NativeCrypto)
}

/// A seeded account: `(address, code)`. Every account gets the same balance.
type SeededAccount = (Address, Vec<u8>);

/// Executes `tx` at `fork` on a state that holds `accounts`, and returns the
/// outcome with the state after it.
fn run(
    accounts: &[SeededAccount],
    tx: &FrameTransaction,
    fork: Fork,
) -> (Result<ExecutionReport, VMError>, GeneralizedDatabase) {
    let store = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let vm_db: DynVmDatabase = Box::new(StoreVmDatabase::new(store, header).unwrap());
    let mut cache: FxHashMap<Address, Account> = FxHashMap::default();
    for (address, code) in accounts {
        cache.insert(
            *address,
            Account::new(
                U256::from(10u64).pow(U256::from(18u64)),
                Code::from_bytecode(Bytes::from(code.clone()), &NativeCrypto),
                tx.nonce_seq,
                FxHashMap::default(),
            ),
        );
    }
    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(vm_db), cache);
    let env = Environment {
        origin: tx.sender,
        gas_limit: tx.max_gas(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(fork, EVMConfig::canonical_values(fork)),
        chain_id: U256::from(CHAIN_ID),
        base_fee_per_gas: U256::from(BASE_FEE),
        gas_price: tx
            .fee_budget(U256::from(BASE_FEE), U256::zero())
            .map_or(U256::from(BASE_FEE), |budget| budget.gas_price),
        tx_nonce: tx.nonce_seq,
        ..Default::default()
    };
    let transaction = Transaction::FrameTransaction(tx.clone());
    let result = VM::new(
        env,
        &mut db,
        &transaction,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("a VM for a frame transaction")
    .execute();
    (result, db)
}

fn is_invalid_format(result: &Result<ExecutionReport, VMError>) -> bool {
    matches!(
        result,
        Err(VMError::TxValidation(
            TxValidationError::InvalidFrameTransactionFormat(_)
        ))
    )
}

#[test]
fn stripping_a_proof_keeps_the_signature_hash_and_changes_the_transaction_hash() {
    let key = key(0x41);
    let tx = proof_transaction(CHAIN_ID, &key, 0, 0);
    let sig_hash = tx.compute_sig_hash();
    let signature = tx.frames[0].proof_bytes().to_vec();
    assert_eq!(signature.len(), SIGNATURE_SIZE);

    let mut stripped = tx.clone();
    let removed = stripped.strip_proofs();

    // The proof left with the claim it proves, the signature hash standing for
    // the zero `msg` the frame encodes.
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].0, (key.public, sig_hash));
    assert_eq!(removed[0].1.as_ref(), signature.as_slice());

    // The frame keeps its claim and the length of its proof.
    assert!(stripped.has_stripped_proofs());
    assert_eq!(
        stripped.stripped_proof_len(0),
        Some(LEANSPHINCS_VERIFIER_PROOF_LENGTH)
    );
    assert_eq!(stripped.frames[0].data, tx.frames[0].data.slice(..64));
    assert_eq!(stripped.frame_data_len(0), tx.frame_data_len(0));
    assert_eq!(tx.frame_data_len(0), 64 + SIGNATURE_SIZE as u64);
    assert_eq!(stripped.stripped_claims(), vec![(key.public, sig_hash)]);
    assert!(!tx.has_stripped_proofs());
    assert!(tx.stripped_claims().is_empty());

    // What was signed did not change: the signature hash already elides the
    // proof to exactly the form the stripped frame carries.
    assert_eq!(stripped.compute_sig_hash(), sig_hash);
    // The transaction hash covers the encoding, which no longer holds the proof.
    assert_ne!(tx_hash(&stripped), tx_hash(&tx));

    // The stripped frame's `data` is the list `[claim, proof_length]`: a
    // 64-byte string and the integer 7740 (0x1e3c).
    let encoded = Transaction::FrameTransaction(stripped.clone()).encode_canonical_to_vec();
    let mut elided = vec![0xf8, 0x45, 0xb8, 0x40];
    elided.extend_from_slice(&stripped.frames[0].data);
    elided.extend_from_slice(&[0x82, 0x1e, 0x3c]);
    assert!(
        encoded
            .windows(elided.len())
            .any(|window| window == elided.as_slice()),
        "the stripped frame is not encoded in its elided form"
    );
    // Stripping again finds nothing left to strip.
    assert!(stripped.strip_proofs().is_empty());
    assert_eq!(stripped.compute_sig_hash(), sig_hash);

    let full = Transaction::FrameTransaction(tx).encode_canonical_to_vec();
    println!(
        "Transaction with its proof: {} bytes. Stripped: {} bytes.",
        full.len(),
        encoded.len()
    );
    // The proof left; the list around the claim and its length took four bytes.
    assert_eq!(full.len() - encoded.len(), SIGNATURE_SIZE - 1);
}

#[test]
fn stripped_transaction_round_trips_through_rlp() {
    let key = key(0x42);
    let tx = proof_transaction(CHAIN_ID, &key, 3, 0);
    let stripped_tx = stripped(&tx);
    let stripped = Transaction::FrameTransaction(stripped_tx.clone());

    let encoded = stripped.encode_canonical_to_vec();
    let decoded = Transaction::decode_canonical(&encoded).expect("a stripped transaction decodes");
    assert_eq!(decoded.encode_canonical_to_vec(), encoded);
    assert_eq!(decoded.hash(&NativeCrypto), stripped.hash(&NativeCrypto));
    let Transaction::FrameTransaction(decoded) = decoded else {
        panic!("a frame transaction decodes as one");
    };
    // Field by field: a transaction caches its encoding, which `==` compares too.
    assert_eq!(decoded.frames, stripped_tx.frames);
    assert_eq!(decoded.stripped_proofs, stripped_tx.stripped_proofs);
    assert_eq!(
        decoded.stripped_proof_len(0),
        Some(LEANSPHINCS_VERIFIER_PROOF_LENGTH)
    );
    assert_eq!(decoded.compute_sig_hash(), tx.compute_sig_hash());

    // The transaction with its proof still decodes as it did, with nothing
    // stripped.
    let full = Transaction::FrameTransaction(tx.clone()).encode_canonical_to_vec();
    let Transaction::FrameTransaction(decoded) = Transaction::decode_canonical(&full).unwrap()
    else {
        panic!("a frame transaction decodes as one");
    };
    assert_eq!(decoded.frames, tx.frames);
    assert!(!decoded.has_stripped_proofs());
}

#[test]
fn stripped_proof_frame_is_reported_with_its_claim_and_the_length_of_its_proof() {
    let key = key(0x4a);
    let tx = proof_transaction(CHAIN_ID, &key, 0, 0);

    let json = serde_json::to_value(Transaction::FrameTransaction(stripped(&tx))).unwrap();
    let frames = json["frames"].as_array().expect("the frames are listed");
    assert_eq!(
        frames[0]["data"],
        serde_json::json!(format!("0x{}", hex::encode(tx.frames[0].proof_claim())))
    );
    assert_eq!(frames[0]["proofLength"], serde_json::json!("0x1e3c"));
    assert!(frames[1].get("proofLength").is_none());

    // A frame that carries its proof reports its data whole.
    let json = serde_json::to_value(Transaction::FrameTransaction(tx.clone())).unwrap();
    assert_eq!(
        json["frames"][0]["data"],
        serde_json::json!(format!("0x{}", hex::encode(&tx.frames[0].data)))
    );
    assert!(json["frames"][0].get("proofLength").is_none());
}

#[test]
fn only_a_proof_frame_holding_its_claim_decodes_with_a_list_for_its_data() {
    let key = key(0x43);
    let tx = proof_transaction(CHAIN_ID, &key, 0, 0);

    // A frame that is not a proof frame has no elided form.
    let mut not_a_proof = tx.clone();
    not_a_proof.stripped_proofs.insert(1, 7740);
    let encoded = Transaction::FrameTransaction(not_a_proof).encode_canonical_to_vec();
    assert!(Transaction::decode_canonical(&encoded).is_err());

    // The list holds the 64-byte claim, not a shorter string.
    let mut short_claim = stripped(&tx);
    short_claim.frames[0].data = short_claim.frames[0].data.slice(..63);
    let encoded = Transaction::FrameTransaction(short_claim).encode_canonical_to_vec();
    assert!(Transaction::decode_canonical(&encoded).is_err());
}

#[test]
fn only_a_proof_the_protocol_may_strip_is_stripped() {
    let key = key(0x44);
    let secp256k1_proof = {
        let mut data = vec![0x11u8; 64];
        data.extend_from_slice(&[0x22u8; 65]);
        Frame {
            mode: u8::from(FrameMode::Proof),
            target: Some(FRAME_TX_SECP256K1_VERIFIER),
            gas_limit: 3_100,
            data: Bytes::from(data),
            ..Default::default()
        }
    };
    let mut short_proof = leansphincs_proof_frame(key.public, H256::zero(), 0);
    short_proof.data = short_proof.data.slice(..64 + SIGNATURE_SIZE - 1);
    let mut tx = transaction_with_proof_frames(
        CHAIN_ID,
        &key,
        0,
        vec![
            // Stripped: a signature for the leanSPHINCS verifier, neither readable
            // nor committed.
            leansphincs_proof_frame(key.public, H256::zero(), 0),
            // Readable: `VERIFY` code may read the bytes, which an aggregate does
            // not supply.
            leansphincs_proof_frame(key.public, H256::zero(), PROOF_READ_FLAG),
            // Committed: the signature hash holds the proof's digest, which the
            // aggregate does not show the proof hashes to.
            leansphincs_proof_frame(key.public, H256::repeat_byte(0x33), PROOF_COMMIT_FLAG),
            // Another verifier's proof.
            secp256k1_proof,
            // Not one signature long.
            short_proof,
            // Stripped: an explicit message.
            leansphincs_proof_frame(key.public, H256::repeat_byte(0x44), 0),
        ],
    );
    let before = tx.clone();
    let sig_hash = tx.compute_sig_hash();

    let removed = tx.strip_proofs();

    assert_eq!(
        tx.stripped_proofs.keys().copied().collect::<Vec<_>>(),
        vec![0, 5]
    );
    assert_eq!(
        removed.iter().map(|(claim, _)| *claim).collect::<Vec<_>>(),
        vec![
            (key.public, sig_hash),
            (key.public, H256::repeat_byte(0x44))
        ]
    );
    for index in 1..=4 {
        assert_eq!(tx.frames[index], before.frames[index]);
    }
    assert_eq!(tx.compute_sig_hash(), sig_hash);
    assert!(tx.validate_static_constraints().is_ok());
    // What the transaction is charged follows the signature hash, not the bytes
    // it carries.
    assert_eq!(tx.max_gas(), before.max_gas());
    assert_eq!(tx.data_cost(), before.data_cost());
    assert_eq!(tx.calldata_floor_total(), before.calldata_floor_total());
}

#[test]
fn gas_is_the_same_before_and_after_lstar_and_with_the_proof_stripped() {
    let key = key(0x45);
    let tx = proof_transaction(CHAIN_ID, &key, 0, 0);
    let stripped = stripped(&tx);
    let accounts = [(key.account, account_code(key.public))];

    // The charge is a function of what the signature hash covers.
    assert_eq!(stripped.max_gas(), tx.max_gas());
    assert_eq!(stripped.data_cost(), tx.data_cost());
    assert_eq!(stripped.calldata_tokens(), tx.calldata_tokens());
    assert_eq!(stripped.calldata_floor_total(), tx.calldata_floor_total());
    assert_eq!(stripped.proof_frames_gas(), tx.proof_frames_gas());

    let (in_hegota, _) = run(&accounts, &tx, Fork::Hegota);
    let (in_lstar, _) = run(&accounts, &tx, Fork::LStar);
    let (stripped_in_lstar, _) = run(&accounts, &stripped, Fork::LStar);
    let in_hegota = in_hegota.expect("the transaction is valid in Hegota");
    let in_lstar = in_lstar.expect("the transaction is valid in LStar with its proof");
    let stripped_in_lstar = stripped_in_lstar.expect("the stripped transaction is valid in LStar");

    // The signature is priced as transaction data, at the calldata floor.
    assert_eq!(in_hegota.gas_used, tx.calldata_floor_total());
    assert_eq!(in_hegota.gas_used, 513_009);
    for report in [&in_lstar, &stripped_in_lstar] {
        assert_eq!(report.gas_used, in_hegota.gas_used);
        assert_eq!(report.gas_spent, in_hegota.gas_spent);
        assert_eq!(report.state_gas_used, in_hegota.state_gas_used);
        assert_eq!(report.payer_address, Some(key.account));
        // Frame by frame: the proof frame is charged its whole limit either way.
        assert_eq!(report.frame_results, in_hegota.frame_results);
    }
    // A frame result is `(status, execution gas used, state gas used, logs)`.
    let proof_frame = &in_hegota.frame_results.as_ref().unwrap()[0];
    assert_eq!(proof_frame.1, LEANSPHINCS_VERIFIER_GAS + WARM_ACCESS);
}

/// Code that stores what it observes of proof frame 0, one word per slot:
/// `PROOFPARAM` key, verifier and msg, `FRAMEPARAM` data length, status, gas used
/// and gas limit, and `TXPARAM` proof count and signature hash.
fn observer_code() -> Vec<u8> {
    let mut code = Vec::new();
    let mut slot = 0u8;
    let mut store = |code: &mut Vec<u8>, read: &[u8]| {
        code.extend_from_slice(read);
        code.extend_from_slice(&[0x60, slot, 0x55]); // PUSH1 slot; SSTORE
        slot += 1;
    };
    for param in [0x00, 0x01, 0x02] {
        store(&mut code, &[0x60, param, 0x60, 0x00, 0xB4]); // PROOFPARAM(param, 0)
    }
    for param in [0x04, 0x05, 0x0A, 0x01] {
        store(&mut code, &[0x60, param, 0x60, 0x00, 0xB3]); // FRAMEPARAM(param, 0)
    }
    for param in [0x0B, 0x08] {
        store(&mut code, &[0x60, param, 0xB0]); // TXPARAM(param)
    }
    code.push(0x00);
    code
}

#[test]
fn stripped_proof_frame_is_observed_like_the_frame_it_replaces() {
    let key = key(0x46);
    let observer = H160([0x0B; 20]);
    let mut tx = transaction_with_proof_frames(
        CHAIN_ID,
        &key,
        0,
        vec![leansphincs_proof_frame(key.public, H256::zero(), 0)],
    );
    // The call frame runs the observer, with state gas for its nine new slots.
    tx.frames[2].target = Some(observer);
    tx.frames[2].gas_limit = 400_000;
    tx.frames[2].state_gas_limit = 2_000_000;
    tx.max_fee = U256::from(10 * GWEI) * U256::from(tx.max_gas());
    sign_proof_frame(&mut tx, 0, &key);
    let stripped = stripped(&tx);
    let accounts = [
        (key.account, account_code(key.public)),
        (observer, observer_code()),
    ];

    let (with_proof, with_proof_db) = run(&accounts, &tx, Fork::LStar);
    let (without_proof, without_proof_db) = run(&accounts, &stripped, Fork::LStar);
    let with_proof = with_proof.expect("the transaction is valid with its proof");
    let without_proof = without_proof.expect("the stripped transaction is valid");
    assert!(
        with_proof
            .frame_results
            .as_ref()
            .unwrap()
            .iter()
            .all(|(status, ..)| *status == 1),
        "every frame succeeds: {:?}",
        with_proof.frame_results
    );
    assert_eq!(without_proof.frame_results, with_proof.frame_results);
    assert_eq!(without_proof.gas_used, with_proof.gas_used);

    let observed = |db: &GeneralizedDatabase| -> Vec<U256> {
        let storage = &db.current_accounts_state[&observer].storage;
        (0..9u64)
            .map(|slot| {
                storage
                    .get(&H256::from_low_u64_be(slot))
                    .copied()
                    .unwrap_or_default()
            })
            .collect()
    };
    let seen = observed(&with_proof_db);
    assert_eq!(observed(&without_proof_db), seen);
    assert_eq!(
        seen,
        vec![
            U256::from_big_endian(key.public.as_bytes()),
            U256::from(0x14),
            // The `msg` as encoded: zero, for the canonical signature hash.
            U256::zero(),
            // The data length the frame had with its proof.
            U256::from(64 + SIGNATURE_SIZE),
            U256::one(),
            U256::from(LEANSPHINCS_VERIFIER_GAS + WARM_ACCESS),
            U256::from(LEANSPHINCS_VERIFIER_GAS + WARM_ACCESS),
            U256::one(),
            U256::from_big_endian(tx.compute_sig_hash().as_bytes()),
        ]
    );
}

#[test]
fn stripped_proof_frame_is_invalid_before_lstar() {
    let key = key(0x47);
    let tx = proof_transaction(CHAIN_ID, &key, 0, 0);
    let accounts = [(key.account, account_code(key.public))];

    let (result, _) = run(&accounts, &stripped(&tx), Fork::Hegota);
    assert!(is_invalid_format(&result), "got {result:?}");
    // The same transaction is valid there with its proof.
    let (result, _) = run(&accounts, &tx, Fork::Hegota);
    result.expect("the transaction is valid in Hegota with its proof");
}

#[test]
fn stripped_proof_frame_must_be_one_the_protocol_may_strip() {
    let key = key(0x48);
    let tx = stripped(&proof_transaction(CHAIN_ID, &key, 0, 0));
    let accounts = [(key.account, account_code(key.public))];
    assert!(tx.validate_static_constraints().is_ok());

    let mut another_verifier = tx.clone();
    another_verifier.frames[0].target = Some(FRAME_TX_SECP256K1_VERIFIER);
    let mut readable = tx.clone();
    readable.frames[0].flags = PROOF_READ_FLAG;
    let mut committed = tx.clone();
    committed.frames[0].flags = PROOF_COMMIT_FLAG;
    // A committed proof needs an explicit message.
    let mut claim = committed.frames[0].data.to_vec();
    claim[32..].fill(0x55);
    committed.frames[0].data = Bytes::from(claim);
    let mut shorter = tx.clone();
    shorter.stripped_proofs.insert(0, 7739);
    let mut longer = tx.clone();
    longer.stripped_proofs.insert(0, 7741);
    let mut empty = tx.clone();
    empty.stripped_proofs.insert(0, 0);
    let mut not_a_proof_frame = tx.clone();
    not_a_proof_frame.stripped_proofs.insert(1, 7740);
    let mut no_such_frame = tx.clone();
    no_such_frame.stripped_proofs.insert(3, 7740);
    let mut more_than_the_claim = tx;
    more_than_the_claim.frames[0].data = Bytes::from(vec![0x66u8; 65]);

    let cases = [
        (another_verifier, "LEANSPHINCS_VERIFIER"),
        (readable, "PROOF_READ_FLAG"),
        (committed, "PROOF_COMMIT_FLAG"),
        (shorter, "7740 bytes"),
        (longer, "7740 bytes"),
        (empty, "7740 bytes"),
        (not_a_proof_frame, "only a proof frame"),
        (no_such_frame, "no such frame"),
        (more_than_the_claim, "claim alone"),
    ];
    for (invalid, reason) in cases {
        let error = invalid
            .validate_static_constraints()
            .expect_err("the stripped frame is not one the protocol may strip");
        assert!(error.contains(reason), "expected `{reason}` in `{error}`");
        assert!(invalid.validate_stripped_proofs().is_err());
        let (result, _) = run(&accounts, &invalid, Fork::LStar);
        assert!(is_invalid_format(&result), "{reason}: got {result:?}");
    }
}

#[test]
fn stripped_proof_frame_needs_a_limit_that_covers_its_verifier() {
    let key = key(0x49);
    let accounts = [(key.account, account_code(key.public))];
    let mut underfunded = transaction_with_proof_frames(
        CHAIN_ID,
        &key,
        0,
        vec![leansphincs_proof_frame(key.public, H256::zero(), 0)],
    );
    underfunded.frames[0].gas_limit = LEANSPHINCS_VERIFIER_GAS + WARM_ACCESS - 1;
    sign_proof_frame(&mut underfunded, 0, &key);
    let is_invalid_proof = |result: &Result<ExecutionReport, VMError>| {
        matches!(
            result,
            Err(VMError::TxValidation(TxValidationError::InvalidFrameProof))
        )
    };

    // With its proof the claim does not hold: the verifier runs out of gas.
    let (result, _) = run(&accounts, &underfunded, Fork::LStar);
    assert!(is_invalid_proof(&result), "got {result:?}");
    // Without it the claim does not hold either: no aggregate stands for a proof
    // whose frame could not have paid for its verifier.
    let (result, _) = run(&accounts, &stripped(&underfunded), Fork::LStar);
    assert!(is_invalid_proof(&result), "got {result:?}");
}
