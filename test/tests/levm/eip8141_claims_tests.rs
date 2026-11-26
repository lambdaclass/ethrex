//! EIP-8141 declarative claims in the VM: the protocol verifiers, `HASCLAIM`,
//! the default code, the pure context custom verifiers run in, and what a claim
//! costs.
//!
//! A transaction's witnesses are not part of it, so executing a transaction
//! never verifies one: a block's witnesses are checked before the block executes
//! and a pooled transaction's when it is admitted. The tests here that run a
//! transaction therefore show what frames see of its claims, and the tests that
//! verify a witness call the verification itself.

use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::types::{
    Account, Claim, Code, Dependency, FRAME_RECEIPT_STATUS_FAILURE, FRAME_RECEIPT_STATUS_SUCCESS,
    Fork, Frame, FrameMode, FrameTransaction, LEANSPHINCS_VERIFIER, P256_VERIFIER,
    SECP256K1_VERIFIER, Transaction,
};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::claims::{verify_dependency, verify_protocol_dependency};
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::{EVMConfig, Environment};
use ethrex_levm::errors::{ExecutionReport, VMError};
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use rustc_hash::FxHashMap;

use super::test_db::TestDatabase;

const CHAIN_ID: u64 = 1;
const BASE_FEE: u64 = 1;
const SENDER: Address = Address::repeat_byte(0xA1);
const RECIPIENT: Address = Address::repeat_byte(0xB2);
const READER: Address = Address::repeat_byte(0xC3);
const VERIFIER: Address = Address::repeat_byte(0x7E);

/// `APPROVE(scope 3)`: approves execution and payment whatever the transaction.
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x5f, 0x5f, 0xaa];

pub(crate) fn push20(address: Address) -> Vec<u8> {
    let mut code = vec![0x73];
    code.extend_from_slice(address.as_bytes());
    code
}

pub(crate) fn push32(word: H256) -> Vec<u8> {
    let mut code = vec![0x7f];
    code.extend_from_slice(word.as_bytes());
    code
}

/// Validation code of an account that approves execution and payment when the
/// transaction carries a claim that `verifier` attests that `key` authorizes the
/// transaction's signature hash, and reverts otherwise.
///
/// ```text
/// require(HASCLAIM(verifier, key, 0))
/// APPROVE(APPROVE_EXECUTION_AND_PAYMENT)
/// ```
pub(crate) fn claim_account_code(verifier: Address, key: H256) -> Vec<u8> {
    // PUSH0 (msg: zero stands for the signature hash); PUSH32 key; PUSH20 verifier
    let mut code = vec![0x5f];
    code.extend_from_slice(&push32(key));
    code.extend_from_slice(&push20(verifier));
    // HASCLAIM
    code.push(0xb4);
    // JUMPI to the approval, 6 bytes ahead: PUSH1 dest, JUMPI, PUSH0, PUSH0, REVERT.
    let approve_at = u8::try_from(code.len() + 6).expect("the code is shorter than 256 bytes");
    code.extend_from_slice(&[0x60, approve_at, 0x57, 0x5f, 0x5f, 0xfd]);
    // JUMPDEST; APPROVE(offset 0, length 0, scope 3)
    code.extend_from_slice(&[0x5b, 0x60, 0x03, 0x5f, 0x5f, 0xaa]);
    code
}

/// Validation code of an account controlled by the leanSPHINCS key `public_key`.
pub(crate) fn leansphincs_account_code(public_key: H256) -> Vec<u8> {
    claim_account_code(LEANSPHINCS_VERIFIER, public_key)
}

fn account(balance: U256, code: &[u8]) -> Account {
    Account::new(
        balance,
        Code::from_bytecode(Bytes::copy_from_slice(code), &NativeCrypto),
        0,
        FxHashMap::default(),
    )
}

fn funded(code: &[u8]) -> Account {
    account(U256::from(10u64).pow(U256::from(18u64)), code)
}

fn contract(code: &[u8]) -> Account {
    account(U256::zero(), code)
}

fn frame_tx(claims: Vec<Claim>, frames: Vec<Frame>) -> FrameTransaction {
    FrameTransaction {
        chain_id: CHAIN_ID,
        sender: SENDER,
        claims,
        frames,
        max_fee: U256::from(10u64).pow(U256::from(17u64)),
        max_priority_fee_per_gas: U256::one(),
        ..Default::default()
    }
}

/// A `VERIFY` frame to the sender that may approve execution and payment.
fn self_verify_frame() -> Frame {
    Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0x03,
        target: None,
        gas_limit: 50_000,
        ..Default::default()
    }
}

/// A `SENDER` frame that calls `target` with `data`.
fn sender_frame(target: Address, data: Vec<u8>) -> Frame {
    Frame {
        mode: u8::from(FrameMode::Sender),
        target: Some(target),
        gas_limit: 200_000,
        state_gas_limit: 1_000_000,
        data: Bytes::from(data),
        ..Default::default()
    }
}

/// Executes `tx` at Hegota over `accounts` and returns its result with the
/// database it ran on.
fn run(
    accounts: Vec<(Address, Account)>,
    tx: FrameTransaction,
) -> (Result<ExecutionReport, VMError>, GeneralizedDatabase) {
    let mut db = GeneralizedDatabase::new_with_account_state(
        Arc::new(TestDatabase::new()),
        accounts.into_iter().collect(),
    );
    let env = Environment {
        origin: tx.sender,
        gas_limit: tx.max_gas(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(CHAIN_ID),
        base_fee_per_gas: U256::from(BASE_FEE),
        gas_price: tx
            .fee_budget(U256::from(BASE_FEE), U256::zero())
            .map_or(U256::from(BASE_FEE), |budget| budget.gas_price),
        tx_nonce: tx.nonce_seq,
        ..Default::default()
    };
    let transaction = Transaction::FrameTransaction(tx);
    let result = VM::new(
        env,
        &mut db,
        &transaction,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("VM::new should succeed for a frame transaction")
    .execute();
    (result, db)
}

fn storage_of(db: &GeneralizedDatabase, address: Address, slot: u64) -> U256 {
    db.current_accounts_state
        .get(&address)
        .and_then(|account| {
            account
                .storage
                .get(&H256(U256::from(slot).to_big_endian()))
                .copied()
        })
        .unwrap_or_default()
}

/// A secp256k1 key and the address it controls.
fn secp256k1_key(seed: u8) -> (k256::ecdsa::SigningKey, Address) {
    let signing_key = k256::ecdsa::SigningKey::from_bytes(&[seed; 32].into()).unwrap();
    let uncompressed = signing_key.verifying_key().to_encoded_point(false);
    let public_key_hash = ethrex_crypto::keccak::keccak_hash(&uncompressed.as_bytes()[1..]);
    (signing_key, Address::from_slice(&public_key_hash[12..]))
}

/// The witness of a `SECP256K1_VERIFIER` claim: `v || r || s`, with `v` the bare
/// recovery id. k256 produces low-s signatures.
fn secp256k1_witness(key: &k256::ecdsa::SigningKey, msg: H256) -> Vec<u8> {
    let (signature, recovery_id) = key.sign_prehash_recoverable(msg.as_bytes()).unwrap();
    let mut witness = vec![recovery_id.to_byte()];
    witness.extend_from_slice(&signature.to_bytes());
    witness
}

/// A P-256 key and the address of its public key, `keccak256(qx || qy)[12:]`.
fn p256_key(seed: u8) -> (p256::ecdsa::SigningKey, Address) {
    let signing_key = p256::ecdsa::SigningKey::from_bytes(&[seed; 32].into()).unwrap();
    let encoded = signing_key.verifying_key().to_encoded_point(false);
    let public_key_hash = ethrex_crypto::keccak::keccak_hash(&encoded.as_bytes()[1..65]);
    (signing_key, Address::from_slice(&public_key_hash[12..]))
}

/// The witness of a `P256_VERIFIER` claim: `r || s || qx || qy`, with `s`
/// normalized to the low half.
fn p256_witness(key: &p256::ecdsa::SigningKey, msg: H256) -> Vec<u8> {
    use p256::ecdsa::signature::hazmat::PrehashSigner;
    let signature: p256::ecdsa::Signature = key.sign_prehash(msg.as_bytes()).unwrap();
    let signature = signature.normalize_s().unwrap_or(signature);
    let encoded = key.verifying_key().to_encoded_point(false);
    let mut witness = signature.to_bytes().to_vec();
    witness.extend_from_slice(&encoded.as_bytes()[1..65]);
    witness
}

/// A leanSPHINCS secret key and its public key.
fn leansphincs_key(seed: u8) -> ([u8; 32], H256) {
    let secret = [seed; 32];
    (secret, H256(lean_sphincs::key_gen(&secret).to_bytes()))
}

fn leansphincs_witness(secret: &[u8; 32], msg: H256) -> Vec<u8> {
    lean_sphincs::sign(secret, msg.as_fixed_bytes())
        .to_bytes()
        .to_vec()
}

fn protocol_dependency(verifier: Address, key: H256, msg: H256) -> Dependency {
    Claim {
        verifier,
        key: Bytes::copy_from_slice(key.as_bytes()),
        msg: Bytes::copy_from_slice(msg.as_bytes()),
        ..Default::default()
    }
    .resolve_dependency(Address::zero(), H256::zero())
    .expect("a well-formed claim")
}

// ==================== Protocol verifiers ====================

mod protocol_verifiers {
    use super::*;
    use ethrex_common::types::ProtocolVerifier;
    use ethrex_levm::claims::{leansphincs_verifier, p256_verifier, secp256k1_verifier};
    use ethrex_levm::precompiles::is_precompile;

    fn secp256k1_input(msg: H256) -> (Address, Vec<u8>) {
        let (key, signer) = secp256k1_key(0x42);
        let mut input = H256::from(signer).as_bytes().to_vec();
        input.extend_from_slice(msg.as_bytes());
        input.extend_from_slice(&secp256k1_witness(&key, msg));
        (signer, input)
    }

    fn p256_input(msg: H256) -> Vec<u8> {
        let (key, address) = p256_key(0x37);
        let mut input = H256::from(address).as_bytes().to_vec();
        input.extend_from_slice(msg.as_bytes());
        input.extend_from_slice(&p256_witness(&key, msg));
        input
    }

    fn leansphincs_input(msg: H256) -> Vec<u8> {
        let (secret, public_key) = leansphincs_key(0x51);
        let mut input = public_key.as_bytes().to_vec();
        input.extend_from_slice(msg.as_bytes());
        input.extend_from_slice(&leansphincs_witness(&secret, msg));
        input
    }

    #[test]
    fn the_protocol_verifiers_fix_their_gas_and_their_witness_length() {
        let table = [
            (SECP256K1_VERIFIER, ProtocolVerifier::Secp256k1, 2_800, 65),
            (P256_VERIFIER, ProtocolVerifier::P256, 6_700, 128),
            (
                LEANSPHINCS_VERIFIER,
                ProtocolVerifier::LeanSphincs,
                13_200,
                7_740,
            ),
        ];
        for (address, verifier, gas, witness_len) in table {
            assert_eq!(ProtocolVerifier::from_address(&address), Some(verifier));
            assert_eq!(verifier.verification_gas(), gas);
            assert_eq!(verifier.witness_len(), witness_len);
        }
        assert_eq!(ProtocolVerifier::from_address(&VERIFIER), None);
    }

    // The three addresses are claim-only: they are outside the precompile range
    // and outside the range EIP-7587 reserves (0x100 to 0x1ff), and no fork makes
    // them precompiles.
    #[test]
    fn the_protocol_verifier_addresses_are_not_precompiles() {
        for address in [SECP256K1_VERIFIER, P256_VERIFIER, LEANSPHINCS_VERIFIER] {
            for fork in [Fork::Amsterdam, Fork::Hegota, Fork::LStar] {
                assert!(!is_precompile(&address, fork, VMType::L1));
            }
            let low = U256::from_big_endian(address.as_bytes());
            assert!(low > U256::from(0x11u64), "above the precompile range");
            assert!(
                low < U256::from(0x100u64) || low > U256::from(0x1ffu64),
                "outside the range reserved for RIPs"
            );
        }
    }

    #[test]
    fn secp256k1_verifier_accepts_the_signer_of_the_message() {
        let (_, input) = secp256k1_input(H256::repeat_byte(0x11));
        assert_eq!(input.len(), 129);
        assert!(secp256k1_verifier(&input, &NativeCrypto));
    }

    #[test]
    fn secp256k1_verifier_rejects_what_does_not_prove_the_claim() {
        let (signer, input) = secp256k1_input(H256::repeat_byte(0x11));
        let rejects = |input: &[u8]| !secp256k1_verifier(input, &NativeCrypto);

        // Another key, another message, a tampered signature.
        let mut other_key = input.clone();
        other_key[31] ^= 1;
        assert!(rejects(&other_key));
        let mut other_msg = input.clone();
        other_msg[63] ^= 1;
        assert!(rejects(&other_msg));
        let mut tampered = input.clone();
        tampered[128] ^= 1;
        assert!(rejects(&tampered));

        // A key that is not an address left-padded with zeros.
        let mut padded = input.clone();
        padded[0] = 1;
        assert!(rejects(&padded));
        assert_eq!(&input[12..32], signer.as_bytes());

        // `v` is the bare recovery id: 27 and 28 are not accepted.
        let mut legacy_v = input.clone();
        legacy_v[64] += 27;
        assert!(rejects(&legacy_v));

        // A zero `r`, and a zero `s`.
        let mut zero_r = input.clone();
        zero_r[65..97].fill(0);
        assert!(rejects(&zero_r));
        let mut zero_s = input.clone();
        zero_s[97..129].fill(0);
        assert!(rejects(&zero_s));

        // Any other length.
        assert!(rejects(&input[..128]));
        let mut longer = input;
        longer.push(0);
        assert!(rejects(&longer));
        assert!(rejects(&[]));
    }

    #[test]
    fn secp256k1_verifier_rejects_the_high_s_form_of_a_valid_signature() {
        // Each signature has one encoding: `s` above n / 2 is rejected even though
        // it recovers the same signer with the other recovery id.
        const SECP256K1_N: [u8; 32] = [
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xfe, 0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c,
            0xd0, 0x36, 0x41, 0x41,
        ];
        let (_, input) = secp256k1_input(H256::repeat_byte(0x11));
        let n = U256::from_big_endian(&SECP256K1_N);
        let s = U256::from_big_endian(&input[97..129]);
        let mut high_s = input;
        high_s[97..129].copy_from_slice(&(n - s).to_big_endian());
        high_s[64] ^= 1;
        assert!(!secp256k1_verifier(&high_s, &NativeCrypto));
    }

    #[test]
    fn p256_verifier_accepts_the_key_that_signed_the_message() {
        let input = p256_input(H256::repeat_byte(0x22));
        assert_eq!(input.len(), 192);
        assert!(p256_verifier(&input, &NativeCrypto));
    }

    #[test]
    fn p256_verifier_rejects_what_does_not_prove_the_claim() {
        let input = p256_input(H256::repeat_byte(0x22));
        let rejects = |input: &[u8]| !p256_verifier(input, &NativeCrypto);

        // The key is the address of the public key the witness carries.
        let mut other_key = input.clone();
        other_key[31] ^= 1;
        assert!(rejects(&other_key));
        let mut other_msg = input.clone();
        other_msg[63] ^= 1;
        assert!(rejects(&other_msg));
        let mut tampered = input.clone();
        tampered[64] ^= 1;
        assert!(rejects(&tampered));

        // The high-s form, which `P256VERIFY` itself accepts.
        const SECP256R1_N: [u8; 32] = [
            0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xbc, 0xe6, 0xfa, 0xad, 0xa7, 0x17, 0x9e, 0x84, 0xf3, 0xb9, 0xca, 0xc2,
            0xfc, 0x63, 0x25, 0x51,
        ];
        let n = U256::from_big_endian(&SECP256R1_N);
        let s = U256::from_big_endian(&input[96..128]);
        let mut high_s = input.clone();
        high_s[96..128].copy_from_slice(&(n - s).to_big_endian());
        assert!(rejects(&high_s));

        // A zero `r`, and any other length.
        let mut zero_r = input.clone();
        zero_r[64..96].fill(0);
        assert!(rejects(&zero_r));
        assert!(rejects(&input[..191]));
        let mut longer = input;
        longer.push(0);
        assert!(rejects(&longer));
    }

    #[test]
    fn leansphincs_verifier_accepts_the_key_that_signed_the_message() {
        let input = leansphincs_input(H256::repeat_byte(0x33));
        assert_eq!(input.len(), 7804);
        assert!(leansphincs_verifier(&input));
    }

    #[test]
    fn leansphincs_verifier_rejects_what_does_not_prove_the_claim() {
        let input = leansphincs_input(H256::repeat_byte(0x33));

        let mut other_key = input.clone();
        other_key[0] ^= 1;
        assert!(!leansphincs_verifier(&other_key));
        let mut other_msg = input.clone();
        other_msg[63] ^= 1;
        assert!(!leansphincs_verifier(&other_msg));
        let mut tampered = input.clone();
        tampered[7803] ^= 1;
        assert!(!leansphincs_verifier(&tampered));

        // The signature has exactly one length.
        assert!(!leansphincs_verifier(&input[..7803]));
        let mut longer = input;
        longer.push(0);
        assert!(!leansphincs_verifier(&longer));
    }

    // `verify_dependency` routes each protocol verifier to its native function
    // and checks the witness length the verifier fixes before anything else.
    #[test]
    fn a_protocol_dependency_is_verified_natively_with_no_state() {
        let msg = H256::repeat_byte(0x44);
        let (secp_key, secp_signer) = secp256k1_key(0x42);
        let (p256_key, p256_address) = p256_key(0x37);
        let (leansphincs_secret, leansphincs_public) = leansphincs_key(0x51);
        let cases = [
            (
                protocol_dependency(SECP256K1_VERIFIER, H256::from(secp_signer), msg),
                secp256k1_witness(&secp_key, msg),
            ),
            (
                protocol_dependency(P256_VERIFIER, H256::from(p256_address), msg),
                p256_witness(&p256_key, msg),
            ),
            (
                protocol_dependency(LEANSPHINCS_VERIFIER, leansphincs_public, msg),
                leansphincs_witness(&leansphincs_secret, msg),
            ),
        ];
        for (dependency, witness) in &cases {
            assert_eq!(
                verify_protocol_dependency(dependency, witness, &NativeCrypto),
                Some(true),
                "{:?}",
                dependency.verifier
            );
            // One byte short, one byte long, and the witness of another message.
            assert_eq!(
                verify_protocol_dependency(
                    dependency,
                    &witness[..witness.len() - 1],
                    &NativeCrypto
                ),
                Some(false)
            );
            let mut longer = witness.clone();
            longer.push(0);
            assert_eq!(
                verify_protocol_dependency(dependency, &longer, &NativeCrypto),
                Some(false)
            );
            let other = Dependency {
                msg: H256::repeat_byte(0x45),
                ..*dependency
            };
            assert_eq!(
                verify_protocol_dependency(&other, witness, &NativeCrypto),
                Some(false)
            );
        }
        // A witness of one verifier does not satisfy another verifier's claim,
        // even when it has the right length for it.
        let leansphincs_as_secp256k1 = Dependency {
            verifier: SECP256K1_VERIFIER,
            ..cases[0].0
        };
        assert_eq!(
            verify_protocol_dependency(&leansphincs_as_secp256k1, &[0x11; 65], &NativeCrypto),
            Some(false)
        );
        // Any other verifier is not the protocol's to evaluate natively.
        let custom = Dependency {
            verifier: VERIFIER,
            ..cases[0].0
        };
        assert_eq!(
            verify_protocol_dependency(&custom, &cases[0].1, &NativeCrypto),
            None
        );
    }
}

// ==================== HASCLAIM ====================

mod hasclaim {
    use super::*;

    /// `SSTORE(0, HASCLAIM(verifier, key, msg) + 1)`, the three operands read from
    /// the call's data in that order.
    const HASCLAIM_READER_CODE: &[u8] = &[
        0x60, 0x40, 0x35, // PUSH1 0x40, CALLDATALOAD (msg)
        0x60, 0x20, 0x35, // PUSH1 0x20, CALLDATALOAD (key)
        0x5f, 0x35, // PUSH0, CALLDATALOAD (verifier)
        0xb4, // HASCLAIM
        0x60, 0x01, 0x01, // PUSH1 1, ADD
        0x5f, 0x55, // PUSH0, SSTORE
        0x00, // STOP
    ];

    fn operands(verifier: H256, key: H256, msg: H256) -> Vec<u8> {
        [verifier.as_bytes(), key.as_bytes(), msg.as_bytes()].concat()
    }

    /// Runs a transaction with `claims` whose second frame asks the reader for
    /// `HASCLAIM(verifier, key, msg)`, and returns the answer: `Some(0)` or
    /// `Some(1)`, or `None` when the reader's frame failed.
    fn has_claim(claims: Vec<Claim>, verifier: H256, key: H256, msg: H256) -> Option<u64> {
        let tx = frame_tx(
            claims,
            vec![
                self_verify_frame(),
                sender_frame(READER, operands(verifier, key, msg)),
            ],
        );
        let (result, db) = run(
            vec![
                (SENDER, funded(APPROVE_BOTH_CODE)),
                (READER, contract(HASCLAIM_READER_CODE)),
            ],
            tx,
        );
        let report = result.expect("the transaction is valid");
        let frames = report.frame_results.expect("frame results");
        if frames[1].0 != FRAME_RECEIPT_STATUS_SUCCESS {
            return None;
        }
        Some(storage_of(&db, READER, 0).low_u64() - 1)
    }

    fn claims() -> Vec<Claim> {
        vec![
            // The sender's own secp256k1 signature of the transaction.
            Claim {
                verifier: SECP256K1_VERIFIER,
                ..Default::default()
            },
            // A leanSPHINCS key's signature of an explicit digest.
            Claim {
                verifier: LEANSPHINCS_VERIFIER,
                key: Bytes::from(vec![0x66; 32]),
                msg: Bytes::from(vec![0x99; 32]),
                ..Default::default()
            },
            // A custom verifier's claim about a 20-byte key.
            Claim {
                verifier: VERIFIER,
                key: Bytes::copy_from_slice(RECIPIENT.as_bytes()),
                gas_limit: 10_000,
                witness_len: 4,
                ..Default::default()
            },
        ]
    }

    #[test]
    fn hasclaim_returns_one_for_a_claim_the_transaction_carries() {
        assert_eq!(
            has_claim(
                claims(),
                H256::from(SECP256K1_VERIFIER),
                H256::from(SENDER),
                H256::zero()
            ),
            Some(1)
        );
        assert_eq!(
            has_claim(
                claims(),
                H256::from(LEANSPHINCS_VERIFIER),
                H256::repeat_byte(0x66),
                H256::repeat_byte(0x99)
            ),
            Some(1)
        );
        // A 20-byte key is compared left-padded.
        assert_eq!(
            has_claim(
                claims(),
                H256::from(VERIFIER),
                H256::from(RECIPIENT),
                H256::zero()
            ),
            Some(1)
        );
    }

    #[test]
    fn hasclaim_returns_zero_unless_all_three_operands_match() {
        let secp = H256::from(SECP256K1_VERIFIER);
        let sender = H256::from(SENDER);
        // Another verifier, another key, another msg.
        assert_eq!(
            has_claim(claims(), H256::from(P256_VERIFIER), sender, H256::zero()),
            Some(0)
        );
        assert_eq!(
            has_claim(claims(), secp, H256::from(RECIPIENT), H256::zero()),
            Some(0)
        );
        assert_eq!(
            has_claim(claims(), secp, sender, H256::repeat_byte(0x99)),
            Some(0)
        );
        // The claim over an explicit digest is not matched by a zero msg.
        assert_eq!(
            has_claim(
                claims(),
                H256::from(LEANSPHINCS_VERIFIER),
                H256::repeat_byte(0x66),
                H256::zero()
            ),
            Some(0)
        );
        // No claims at all.
        assert_eq!(has_claim(Vec::new(), secp, sender, H256::zero()), Some(0));
    }

    // A zero msg stands for the signature hash; the hash itself matches nothing,
    // because `msg` is compared with the claim as encoded.
    #[test]
    fn hasclaim_does_not_match_the_signature_hash_spelled_out() {
        let tx = frame_tx(claims(), vec![self_verify_frame()]);
        // The reader's frame is part of the hash, so the hash of the transaction
        // actually run differs; any non-zero msg makes the point.
        let sig_hash = tx.compute_sig_hash();
        assert_eq!(
            has_claim(
                claims(),
                H256::from(SECP256K1_VERIFIER),
                H256::from(SENDER),
                sig_hash
            ),
            Some(0)
        );
    }

    // `verifier` is an address in a word: with any of its upper 12 bytes set it
    // matches nothing, rather than being truncated to an address that might.
    #[test]
    fn hasclaim_returns_zero_for_a_verifier_wider_than_an_address() {
        let mut wide = H256::from(SECP256K1_VERIFIER);
        wide.0[0] = 1;
        assert_eq!(
            has_claim(claims(), wide, H256::from(SENDER), H256::zero()),
            Some(0)
        );
        let mut wide = H256::from(SECP256K1_VERIFIER);
        wide.0[11] = 1;
        assert_eq!(
            has_claim(claims(), wide, H256::from(SENDER), H256::zero()),
            Some(0)
        );
    }

    /// The execution gas the transaction's second frame uses when it runs `code`.
    fn frame_gas(code: &[u8]) -> u64 {
        let tx = frame_tx(
            claims(),
            vec![self_verify_frame(), sender_frame(READER, Vec::new())],
        );
        let (result, _) = run(
            vec![
                (SENDER, funded(APPROVE_BOTH_CODE)),
                (READER, contract(code)),
            ],
            tx,
        );
        let frames = result
            .expect("the transaction is valid")
            .frame_results
            .expect("frame results");
        assert_eq!(frames[1].0, FRAME_RECEIPT_STATUS_SUCCESS);
        frames[1].1
    }

    #[test]
    fn hasclaim_costs_three_gas_and_takes_three_operands() {
        // PUSH0 x3, HASCLAIM, POP, STOP against PUSH0 x3, POP x3, PUSH0, POP, STOP:
        // the same stack traffic apart from HASCLAIM itself, which pops three
        // words and pushes one. POP and PUSH0 cost 2 each.
        let with_hasclaim = frame_gas(&[0x5f, 0x5f, 0x5f, 0xb4, 0x50, 0x00]);
        let without = frame_gas(&[0x5f, 0x5f, 0x5f, 0x50, 0x50, 0x50, 0x5f, 0x50, 0x00]);
        assert_eq!(with_hasclaim + 3 * 2 + 2, without + 3);
    }

    #[test]
    fn hasclaim_with_fewer_than_three_operands_halts() {
        let tx = frame_tx(
            claims(),
            vec![self_verify_frame(), sender_frame(READER, Vec::new())],
        );
        let (result, _) = run(
            vec![
                (SENDER, funded(APPROVE_BOTH_CODE)),
                // PUSH0, PUSH0, HASCLAIM
                (READER, contract(&[0x5f, 0x5f, 0xb4, 0x00])),
            ],
            tx,
        );
        let frames = result.unwrap().frame_results.unwrap();
        assert_eq!(frames[1].0, FRAME_RECEIPT_STATUS_FAILURE);
        // An exceptional halt consumes the frame's whole execution limit.
        assert_eq!(frames[1].1, 200_000);
    }

    // The instruction that used to sit after 0xb4 is gone: 0xb5 is not an
    // instruction, so it halts like any undefined opcode.
    #[test]
    fn opcode_0xb5_is_not_an_instruction() {
        let tx = frame_tx(
            claims(),
            vec![self_verify_frame(), sender_frame(READER, Vec::new())],
        );
        let (result, _) = run(
            vec![
                (SENDER, funded(APPROVE_BOTH_CODE)),
                (READER, contract(&[0x5f, 0x5f, 0x5f, 0x5f, 0xb5, 0x00])),
            ],
            tx,
        );
        let frames = result.unwrap().frame_results.unwrap();
        assert_eq!(frames[1].0, FRAME_RECEIPT_STATUS_FAILURE);
    }
}

// ==================== Accounts that authorize through a claim ====================

mod authorization {
    use super::*;

    #[test]
    fn a_leansphincs_account_approves_on_its_claim() {
        let (secret, public_key) = leansphincs_key(0x51);
        let claim = Claim {
            verifier: LEANSPHINCS_VERIFIER,
            key: Bytes::copy_from_slice(public_key.as_bytes()),
            ..Default::default()
        };
        let mut transfer = sender_frame(RECIPIENT, Vec::new());
        transfer.value = U256::from(1_000u64);
        let tx = frame_tx(vec![claim], vec![self_verify_frame(), transfer]);
        let sig_hash = tx.compute_sig_hash();
        let dependencies = tx.dependencies().expect("a well-formed claim");

        let (result, db) = run(
            vec![
                (SENDER, funded(&leansphincs_account_code(public_key))),
                (RECIPIENT, funded(&[])),
            ],
            tx,
        );
        let report = result.expect("the account approves on its claim");
        assert_eq!(report.payer_address, Some(SENDER));
        let frames = report.frame_results.expect("frame results");
        assert_eq!(frames[0].0, FRAME_RECEIPT_STATUS_SUCCESS);
        assert_eq!(frames[1].0, FRAME_RECEIPT_STATUS_SUCCESS);
        assert_eq!(
            db.current_accounts_state[&RECIPIENT].info.balance,
            U256::from(10u64).pow(U256::from(18u64)) + U256::from(1_000u64)
        );

        // The claim resolves to one dependency, which the account's signature of
        // the transaction's signature hash satisfies and no other signature does.
        assert_eq!(
            dependencies,
            vec![protocol_dependency(
                LEANSPHINCS_VERIFIER,
                public_key,
                sig_hash
            )]
        );
        let witness = leansphincs_witness(&secret, sig_hash);
        assert_eq!(
            verify_protocol_dependency(&dependencies[0], &witness, &NativeCrypto),
            Some(true)
        );
        let (other_secret, _) = leansphincs_key(0x52);
        assert_eq!(
            verify_protocol_dependency(
                &dependencies[0],
                &leansphincs_witness(&other_secret, sig_hash),
                &NativeCrypto
            ),
            Some(false)
        );
        assert_eq!(
            verify_protocol_dependency(
                &dependencies[0],
                &leansphincs_witness(&secret, H256::repeat_byte(0x01)),
                &NativeCrypto
            ),
            Some(false)
        );
    }

    /// Whether a transaction with `claims` from a leanSPHINCS account is valid.
    fn leansphincs_account_accepts(public_key: H256, claims: Vec<Claim>) -> bool {
        let tx = frame_tx(claims, vec![self_verify_frame()]);
        run(
            vec![(SENDER, funded(&leansphincs_account_code(public_key)))],
            tx,
        )
        .0
        .is_ok()
    }

    #[test]
    fn a_leansphincs_account_rejects_a_transaction_without_its_claim() {
        let (_, public_key) = leansphincs_key(0x51);
        let (_, other_key) = leansphincs_key(0x52);
        let key = |key: H256| Bytes::copy_from_slice(key.as_bytes());
        let own = Claim {
            verifier: LEANSPHINCS_VERIFIER,
            key: key(public_key),
            ..Default::default()
        };
        assert!(leansphincs_account_accepts(public_key, vec![own.clone()]));

        // No claim, a claim under another key, a claim over another message, and a
        // claim another verifier checks.
        assert!(!leansphincs_account_accepts(public_key, Vec::new()));
        assert!(!leansphincs_account_accepts(
            public_key,
            vec![Claim {
                key: key(other_key),
                ..own.clone()
            }]
        ));
        assert!(!leansphincs_account_accepts(
            public_key,
            vec![Claim {
                msg: Bytes::from(vec![0x99; 32]),
                ..own.clone()
            }]
        ));
        assert!(!leansphincs_account_accepts(
            public_key,
            vec![Claim {
                verifier: VERIFIER,
                gas_limit: 10_000,
                ..own.clone()
            }]
        ));
        // Its claim among others, in any position.
        assert!(leansphincs_account_accepts(
            public_key,
            vec![
                Claim {
                    key: key(other_key),
                    ..own.clone()
                },
                Claim {
                    verifier: SECP256K1_VERIFIER,
                    ..Default::default()
                },
                own
            ]
        ));
    }

    // ---- Default code ----

    /// Runs `[VERIFY]` from a sender without code, whose default code decides.
    fn run_default_code(
        claims: Vec<Claim>,
        verify: Frame,
    ) -> (Result<ExecutionReport, VMError>, GeneralizedDatabase) {
        run(vec![(SENDER, funded(&[]))], frame_tx(claims, vec![verify]))
    }

    fn sender_secp256k1_claim() -> Claim {
        Claim {
            verifier: SECP256K1_VERIFIER,
            ..Default::default()
        }
    }

    #[test]
    fn default_code_approves_on_a_secp256k1_claim_for_the_account() {
        let (result, db) = run_default_code(vec![sender_secp256k1_claim()], self_verify_frame());
        let report = result.expect("the default code approves");
        assert_eq!(report.payer_address, Some(SENDER));
        assert_eq!(db.current_accounts_state[&SENDER].info.nonce, 1);
        // The default code draws no execution gas of its own: the frame's only
        // charge is its target's access, and the sender is warm.
        let frames = report.frame_results.expect("frame results");
        assert_eq!(frames[0].1, 100);

        // The same claim with the sender spelled out as a 20-byte key.
        let spelled_out = Claim {
            key: Bytes::copy_from_slice(SENDER.as_bytes()),
            ..sender_secp256k1_claim()
        };
        assert!(
            run_default_code(vec![spelled_out], self_verify_frame())
                .0
                .is_ok()
        );
    }

    #[test]
    fn default_code_reverts_when_the_frame_carries_data() {
        let mut verify = self_verify_frame();
        verify.data = Bytes::from_static(&[0x00]);
        assert!(
            run_default_code(vec![sender_secp256k1_claim()], verify)
                .0
                .is_err()
        );
    }

    #[test]
    fn default_code_reverts_without_a_secp256k1_claim_over_the_signature_hash() {
        let rejected =
            |claims: Vec<Claim>| run_default_code(claims, self_verify_frame()).0.is_err();
        assert!(rejected(Vec::new()));
        // Another account's signature.
        assert!(rejected(vec![Claim {
            key: Bytes::copy_from_slice(RECIPIENT.as_bytes()),
            ..sender_secp256k1_claim()
        }]));
        // The account's signature of an explicit digest, which says nothing about
        // this transaction's frames.
        assert!(rejected(vec![Claim {
            msg: Bytes::from(vec![0x99; 32]),
            ..sender_secp256k1_claim()
        }]));
        // A claim for the account under any other verifier.
        assert!(rejected(vec![Claim {
            verifier: P256_VERIFIER,
            ..Default::default()
        }]));
        assert!(rejected(vec![Claim {
            verifier: VERIFIER,
            gas_limit: 10_000,
            ..Default::default()
        }]));
    }

    #[test]
    fn default_code_reverts_when_the_frame_allows_no_scope() {
        let mut verify = self_verify_frame();
        verify.flags = 0;
        assert!(
            run_default_code(vec![sender_secp256k1_claim()], verify)
                .0
                .is_err()
        );
    }

    // A sponsor without code approves payment on a claim for its own address,
    // wherever that claim sits among the transaction's claims.
    #[test]
    fn default_code_of_a_sponsor_approves_payment_on_its_own_claim() {
        let sponsor = RECIPIENT;
        let sponsor_claim = Claim {
            verifier: SECP256K1_VERIFIER,
            key: Bytes::copy_from_slice(sponsor.as_bytes()),
            ..Default::default()
        };
        let run_sponsored = |claims: Vec<Claim>| {
            let only_verify = Frame {
                flags: 0x02,
                ..self_verify_frame()
            };
            let pay = Frame {
                flags: 0x01,
                target: Some(sponsor),
                ..self_verify_frame()
            };
            run(
                vec![(SENDER, funded(&[])), (sponsor, funded(&[]))],
                frame_tx(claims, vec![only_verify, pay]),
            )
            .0
        };
        for claims in [
            vec![sender_secp256k1_claim(), sponsor_claim.clone()],
            vec![sponsor_claim, sender_secp256k1_claim()],
        ] {
            let report = run_sponsored(claims).expect("both default codes approve");
            assert_eq!(report.payer_address, Some(sponsor));
        }
        // Without the sponsor's claim nobody pays, and the sender's claim does
        // not stand in for it.
        assert!(run_sponsored(vec![sender_secp256k1_claim()]).is_err());
    }
}

// ==================== Claim-only addresses seen from the EVM ====================

mod claim_only_addresses {
    use super::*;

    /// `STATICCALL` the address in the call's data with no input, then
    /// `SSTORE(0, success + 1)` and `SSTORE(1, RETURNDATASIZE + 1)`.
    const CALLER_CODE: &[u8] = &[
        0x5f, 0x5f, 0x5f, 0x5f, // retSize, retOffset, argsSize, argsOffset
        0x5f, 0x35, // PUSH0, CALLDATALOAD (address)
        0x5a, 0xfa, // GAS, STATICCALL
        0x60, 0x01, 0x01, 0x5f, 0x55, // PUSH1 1, ADD, PUSH0, SSTORE
        0x3d, 0x60, 0x01, 0x01, 0x60, 0x01, 0x55, // RETURNDATASIZE + 1 into slot 1
        0x00,
    ];

    /// Calls `callee` from a contract and returns `(success, returndatasize,
    /// execution gas of the frame)`.
    fn call(callee: Address) -> (u64, u64, u64) {
        let tx = frame_tx(
            Vec::new(),
            vec![
                self_verify_frame(),
                sender_frame(READER, H256::from(callee).as_bytes().to_vec()),
            ],
        );
        let (result, db) = run(
            vec![
                (SENDER, funded(APPROVE_BOTH_CODE)),
                (READER, contract(CALLER_CODE)),
            ],
            tx,
        );
        let frames = result.expect("valid").frame_results.expect("frame results");
        assert_eq!(frames[1].0, FRAME_RECEIPT_STATUS_SUCCESS);
        (
            storage_of(&db, READER, 0).low_u64() - 1,
            storage_of(&db, READER, 1).low_u64() - 1,
            frames[1].1,
        )
    }

    // EIP-8141 §Protocol Verifiers: calling one of the three addresses from EVM
    // code behaves as calling an account without code. In particular it is not
    // warm the way a precompile is.
    #[test]
    fn calling_a_protocol_verifier_is_calling_an_account_without_code() {
        let empty_account = call(Address::repeat_byte(0xE0));
        assert_eq!((empty_account.0, empty_account.1), (1, 0));
        for verifier in [SECP256K1_VERIFIER, P256_VERIFIER, LEANSPHINCS_VERIFIER] {
            assert_eq!(call(verifier), empty_account, "{verifier:?}");
        }
        // A precompile is warm from the start: the call to the identity
        // precompile, 15 gas of its own, costs less than the cold access alone.
        let identity = call(Address::from_low_u64_be(4));
        assert!(identity.2 < empty_account.2);
    }
}

// ==================== Pure context ====================

mod pure_context {
    use super::*;
    use ethrex_levm::gas_cost::cold_account_access_cost;

    const GAS: u64 = 1_000_000;
    /// `STOP`: a verifier that accepts every witness.
    const ACCEPT_ALL: &[u8] = &[0x00];
    /// `PUSH0, PUSH0, REVERT`: a verifier that accepts none.
    const REJECT_ALL: &[u8] = &[0x5f, 0x5f, 0xfd];

    fn hegota() -> EVMConfig {
        EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota))
    }

    /// A database whose state, as a block's pre-state would be, holds `accounts`.
    fn state(accounts: Vec<(Address, Account)>) -> GeneralizedDatabase {
        GeneralizedDatabase::new(Arc::new(TestDatabase {
            accounts: accounts.into_iter().collect(),
        }))
    }

    fn dependency(verifier: Address, gas_limit: u64, witness_len: u64) -> Dependency {
        Dependency {
            verifier,
            key: H256::repeat_byte(0x0b),
            msg: H256::repeat_byte(0x0c),
            gas_limit,
            witness_len,
        }
    }

    fn holds(db: &mut GeneralizedDatabase, dependency: &Dependency, witness: &[u8]) -> bool {
        verify_dependency(dependency, witness, db, hegota(), VMType::L1, &NativeCrypto)
            .expect("verification runs")
    }

    /// Whether a verifier running `code` accepts an empty witness with `GAS`.
    fn code_holds(code: &[u8]) -> bool {
        let mut db = state(vec![(VERIFIER, contract(code))]);
        holds(&mut db, &dependency(VERIFIER, GAS, 0), &[])
    }

    #[test]
    fn a_contract_verifier_decides_its_claim() {
        assert!(code_holds(ACCEPT_ALL));
        assert!(!code_holds(REJECT_ALL));
        // INVALID: an exceptional halt rejects too.
        assert!(!code_holds(&[0xfe]));
        // The call succeeds whatever it returns: RETURN of a zero word.
        assert!(code_holds(&[0x60, 0x20, 0x5f, 0xf3]));
    }

    /// A verifier that succeeds if and only if its whole input hashes to `hash`.
    fn expects_input(hash: H256) -> Vec<u8> {
        // CALLDATACOPY(0, 0, CALLDATASIZE); KECCAK256(0, CALLDATASIZE)
        let mut code = vec![0x36, 0x5f, 0x5f, 0x37, 0x36, 0x5f, 0x20];
        code.extend_from_slice(&push32(hash));
        code.push(0x14); // EQ
        let accept_at = u8::try_from(code.len() + 6).unwrap();
        code.extend_from_slice(&[0x60, accept_at, 0x57, 0x5f, 0x5f, 0xfd, 0x5b, 0x00]);
        code
    }

    // The verifier's input is `key || msg || witness`, and nothing else.
    #[test]
    fn a_contract_verifier_receives_the_key_the_message_and_the_witness() {
        let witness = vec![0xD7; 100];
        let dependency = dependency(VERIFIER, GAS, 100);
        let input = [
            dependency.key.as_bytes(),
            dependency.msg.as_bytes(),
            &witness,
        ]
        .concat();
        let code = expects_input(H256(ethrex_crypto::keccak::keccak_hash(&input)));
        let mut db = state(vec![(VERIFIER, contract(&code))]);

        assert!(holds(&mut db, &dependency, &witness));
        // Another witness of the same length, another key, another message.
        let mut other_witness = witness.clone();
        other_witness[99] ^= 1;
        assert!(!holds(&mut db, &dependency, &other_witness));
        let other_key = Dependency {
            key: H256::repeat_byte(0x0d),
            ..dependency
        };
        assert!(!holds(&mut db, &other_key, &witness));
        let other_msg = Dependency {
            msg: H256::repeat_byte(0x0d),
            ..dependency
        };
        assert!(!holds(&mut db, &other_msg, &witness));
    }

    // The witness has exactly the length the claim declares: the verifier is not
    // even run on any other.
    #[test]
    fn a_witness_of_another_length_than_declared_never_satisfies() {
        let mut db = state(vec![(VERIFIER, contract(ACCEPT_ALL))]);
        let dependency = dependency(VERIFIER, GAS, 4);
        assert!(holds(&mut db, &dependency, &[1, 2, 3, 4]));
        assert!(!holds(&mut db, &dependency, &[1, 2, 3]));
        assert!(!holds(&mut db, &dependency, &[1, 2, 3, 4, 5]));
        assert!(!holds(&mut db, &dependency, &[]));
    }

    // A verifier is called cold, with its entry access drawn from the claim's gas
    // limit: `JUMPDEST x3, STOP` needs the cold account access and 3 gas more.
    #[test]
    fn a_contract_verifier_pays_its_cold_access_from_the_claims_gas_limit() {
        let cold = cold_account_access_cost(Fork::Hegota);
        let mut db = state(vec![(VERIFIER, contract(&[0x5b, 0x5b, 0x5b, 0x00]))]);
        assert!(holds(&mut db, &dependency(VERIFIER, cold + 3, 0), &[]));
        assert!(!holds(&mut db, &dependency(VERIFIER, cold + 2, 0), &[]));
        assert!(!holds(&mut db, &dependency(VERIFIER, cold - 1, 0), &[]));
        assert!(!holds(&mut db, &dependency(VERIFIER, 1, 0), &[]));
        // Verifying once does not warm the verifier for the next verification.
        assert!(!holds(&mut db, &dependency(VERIFIER, cold + 2, 0), &[]));
        assert!(holds(&mut db, &dependency(VERIFIER, cold + 3, 0), &[]));
    }

    #[test]
    fn an_account_without_code_is_not_a_verifier() {
        let mut db = state(vec![(VERIFIER, funded(&[]))]);
        assert!(!holds(&mut db, &dependency(VERIFIER, GAS, 0), &[]));
        // Nor is an address the state does not have: a verifier has to be in the
        // state the claim is verified against.
        let mut db = state(Vec::new());
        assert!(!holds(&mut db, &dependency(VERIFIER, GAS, 0), &[]));
    }

    // Code that is an EIP-7702 delegation indicator can change, so an account
    // holding one is not a verifier even when it delegates to one.
    #[test]
    fn a_delegated_account_is_not_a_verifier() {
        let delegate = Address::repeat_byte(0x7F);
        let mut indicator = vec![0xef, 0x01, 0x00];
        indicator.extend_from_slice(delegate.as_bytes());
        let mut db = state(vec![
            (VERIFIER, contract(&indicator)),
            (delegate, contract(ACCEPT_ALL)),
        ]);
        assert!(!holds(&mut db, &dependency(VERIFIER, GAS, 0), &[]));
        assert!(holds(&mut db, &dependency(delegate, GAS, 0), &[]));
    }

    /// Seven zero words, then `opcode`, then `STOP`: enough operands for any
    /// instruction, all of them harmless.
    fn probe(opcode: u8) -> Vec<u8> {
        let mut code = vec![0x5f; 7];
        code.extend_from_slice(&[opcode, 0x00]);
        code
    }

    /// Runs `code` in an ordinary `SENDER` frame and reports whether it succeeds.
    fn runs_in_an_ordinary_frame(code: &[u8]) -> bool {
        let tx = frame_tx(
            Vec::new(),
            vec![self_verify_frame(), sender_frame(VERIFIER, Vec::new())],
        );
        let (result, _) = run(
            vec![
                (SENDER, funded(APPROVE_BOTH_CODE)),
                (VERIFIER, contract(code)),
            ],
            tx,
        );
        result.expect("valid").frame_results.expect("frame results")[1].0
            == FRAME_RECEIPT_STATUS_SUCCESS
    }

    // EIP-8141 §Pure Context: every instruction that reads the call's context,
    // other accounts, the chain, the block or the transaction halts. Each one
    // runs to completion in an ordinary frame, so it is the pure context that
    // halts it.
    #[test]
    fn the_pure_context_halts_on_what_reads_beyond_the_input() {
        let mut banned: Vec<u8> = Vec::new();
        banned.extend(0x30..=0x33); // ADDRESS, BALANCE, ORIGIN, CALLER
        banned.extend(0x3a..=0x3c); // GASPRICE, EXTCODESIZE, EXTCODECOPY
        banned.push(0x3f); // EXTCODEHASH
        banned.extend(0x40..=0x4b); // BLOCKHASH through SLOTNUM
        banned.push(0x54); // SLOAD
        banned.push(0x5c); // TLOAD
        banned.extend(0xb0..=0xb4); // TXPARAM through HASCLAIM
        banned.extend([0xf1, 0xf2, 0xf4]); // CALL, CALLCODE, DELEGATECALL
        for opcode in banned {
            let code = probe(opcode);
            assert!(
                runs_in_an_ordinary_frame(&code),
                "opcode {opcode:#04x} runs in an ordinary frame"
            );
            assert!(
                !code_holds(&code),
                "opcode {opcode:#04x} halts in the pure context"
            );
        }
        // APPROVE too. In an ordinary frame it needs a scope, so it has no control
        // run here; the pure context halts it before it looks at one.
        assert!(!code_holds(&probe(0xaa)));
        // The instructions later EIPs add are not permitted either: EIP-7906's
        // TXTRACE and EVENTDATACOPY, and EIP-8298's SETCODEFROM.
        for opcode in [0xb6, 0xb7, 0xa7] {
            assert!(!code_holds(&probe(opcode)), "opcode {opcode:#04x}");
        }
    }

    // The pure context is a static call: what `STATICCALL` forbids stays
    // forbidden.
    #[test]
    fn the_pure_context_is_static() {
        // SSTORE, TSTORE, LOG0, CREATE, CREATE2, SELFDESTRUCT
        for opcode in [0x55, 0x5d, 0xa0, 0xf0, 0xf5, 0xff] {
            assert!(!code_holds(&probe(opcode)), "opcode {opcode:#04x}");
        }
    }

    // What computes on the input, the verifier's own code and its gas stays
    // available, CALLVALUE included, since it is always zero.
    #[test]
    fn the_pure_context_keeps_what_computes_on_the_input() {
        // CALLVALUE, CALLDATALOAD, CALLDATASIZE, CALLDATACOPY, CODESIZE, CODECOPY,
        // RETURNDATASIZE, RETURNDATACOPY, KECCAK256, MLOAD, MSTORE, PC, MSIZE, GAS,
        // ADD, and EIP-8024's DUPN-free neighbours are covered by the ordinary
        // tables.
        for opcode in [
            0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3d, 0x3e, 0x20, 0x51, 0x52, 0x58, 0x59, 0x5a,
            0x01,
        ] {
            assert!(code_holds(&probe(opcode)), "opcode {opcode:#04x}");
        }
        // CALLVALUE is zero: `CALLVALUE, ISZERO, PUSH1 6, JUMPI, INVALID, JUMPDEST, STOP`.
        assert!(code_holds(&[
            0x34, 0x15, 0x60, 0x06, 0x57, 0xfe, 0x5b, 0x00
        ]));
    }

    /// `STATICCALL(GAS, callee, 0, 0, 0, 0)`, then `STOP` whatever it returned.
    fn staticcall_code(callee: Address) -> Vec<u8> {
        let mut code = vec![0x5f, 0x5f, 0x5f, 0x5f];
        code.extend_from_slice(&push20(callee));
        code.extend_from_slice(&[0x5a, 0xfa, 0x50, 0x00]);
        code
    }

    #[test]
    fn the_pure_context_allows_a_staticcall_only_to_a_precompile() {
        let helper = Address::repeat_byte(0x7F);
        let verify = |callee: Address| {
            let mut db = state(vec![
                (VERIFIER, contract(&staticcall_code(callee))),
                (helper, contract(ACCEPT_ALL)),
            ]);
            holds(&mut db, &dependency(VERIFIER, GAS, 0), &[])
        };
        // SHA-256, the identity precompile, and P256VERIFY at 0x100.
        assert!(verify(Address::from_low_u64_be(2)));
        assert!(verify(Address::from_low_u64_be(4)));
        assert!(verify(Address::from_low_u64_be(0x100)));
        // A contract, an account without code, and the claim-only protocol
        // verifier addresses, which are not precompiles.
        assert!(!verify(helper));
        assert!(!verify(Address::repeat_byte(0xE0)));
        assert!(!verify(SECP256K1_VERIFIER));
        assert!(!verify(LEANSPHINCS_VERIFIER));
    }

    // A verifier that is itself an active precompile is dispatched directly, warm,
    // and holds exactly when the precompile succeeds. BLAKE2F fails on a
    // malformed input: it takes 213 bytes whose last one is 0 or 1, and costs one
    // gas per round, here zero rounds since the key opens with four zero bytes.
    #[test]
    fn a_precompile_verifier_holds_when_the_precompile_succeeds() {
        let blake2f = Address::from_low_u64_be(9);
        let mut key = H256::zero();
        key.0[31] = 1;
        let dependency = |gas_limit: u64, witness_len: u64| Dependency {
            verifier: blake2f,
            key,
            msg: H256::repeat_byte(0x0c),
            gas_limit,
            witness_len,
        };
        let mut witness = vec![0u8; 149];
        witness[148] = 1;
        let mut db = state(Vec::new());

        // The warm access, 100 gas, comes out of the claim's limit.
        assert!(holds(&mut db, &dependency(100, 149), &witness));
        assert!(!holds(&mut db, &dependency(99, 149), &witness));
        // A final-block flag that is neither 0 nor 1, and an input of another
        // length, make the precompile fail.
        let mut malformed = witness.clone();
        malformed[148] = 2;
        assert!(!holds(&mut db, &dependency(GAS, 149), &malformed));
        assert!(!holds(&mut db, &dependency(GAS, 148), &witness[..148]));
    }

    // Verification reads the verifier's code and nothing else: it leaves no state
    // change behind for the block that follows.
    #[test]
    fn verification_changes_no_state() {
        let mut db = state(vec![(VERIFIER, contract(ACCEPT_ALL))]);
        assert!(holds(&mut db, &dependency(VERIFIER, GAS, 0), &[]));
        let transitions = ethrex_vm::backends::levm::LEVM::get_state_transitions(&mut db)
            .expect("state transitions");
        assert!(transitions.is_empty(), "{transitions:?}");
    }
}

// ==================== What a claim costs ====================

mod gas {
    use super::*;

    const BURNER: Address = Address::repeat_byte(0xD4);

    /// The gas a transaction with `claims` is charged, over a sender that
    /// approves anything and a frame that burns enough execution gas for the
    /// calldata floor not to bind.
    fn gas_used(claims: Vec<Claim>) -> u64 {
        let mut burner_code = vec![0x5b; 20_000];
        burner_code.push(0x00);
        let tx = frame_tx(
            claims,
            vec![self_verify_frame(), sender_frame(BURNER, Vec::new())],
        );
        assert!(
            tx.calldata_floor_total() < tx.mandatory_gas() + tx.data_cost() + 20_000,
            "the floor must not bind for the difference to be the claim's cost"
        );
        let (result, _) = run(
            vec![
                (SENDER, funded(APPROVE_BOTH_CODE)),
                (BURNER, contract(&burner_code)),
            ],
            tx,
        );
        let report = result.expect("valid");
        let frames = report.frame_results.as_ref().expect("frame results");
        assert_eq!(frames[1].0, FRAME_RECEIPT_STATUS_SUCCESS);
        report.gas_used
    }

    // The verifier's 20-byte address is priced as calldata: 19 zero bytes at 4
    // gas and one non-zero byte at 16.
    const VERIFIER_ADDRESS_COST: u64 = 19 * 4 + 16;

    #[test]
    fn a_protocol_claim_is_charged_its_verification_and_its_witness_bytes() {
        let base = gas_used(Vec::new());
        // secp256k1, with the default key and message: 2,800 to verify and 4 gas
        // for each of the witness's 65 bytes.
        let secp256k1 = Claim {
            verifier: SECP256K1_VERIFIER,
            ..Default::default()
        };
        assert_eq!(
            gas_used(vec![secp256k1]) - base,
            2_800 + 4 * 65 + VERIFIER_ADDRESS_COST
        );
        // P-256 with a 20-byte key: 6,700 and 128 witness bytes.
        let p256 = Claim {
            verifier: P256_VERIFIER,
            key: Bytes::from(vec![0x05; 20]),
            ..Default::default()
        };
        assert_eq!(
            gas_used(vec![p256]) - base,
            6_700 + 4 * 128 + VERIFIER_ADDRESS_COST + 20 * 16
        );
        // leanSPHINCS with its 32-byte key: 13,200 and 7,740 witness bytes.
        let leansphincs = Claim {
            verifier: LEANSPHINCS_VERIFIER,
            key: Bytes::from(vec![0x06; 32]),
            ..Default::default()
        };
        assert_eq!(
            gas_used(vec![leansphincs]) - base,
            13_200 + 4 * 7_740 + VERIFIER_ADDRESS_COST + 32 * 16
        );
    }

    // EIP-8141 §Pure Context: the full `claim.gas_limit` is always charged,
    // whatever the verifier would use, and the witness is priced by its declared
    // length as non-zero calldata. Neither depends on the witness bytes, which
    // the transaction does not even carry.
    #[test]
    fn a_custom_claim_is_charged_its_whole_gas_limit_and_its_declared_witness_length() {
        let base = gas_used(Vec::new());
        let custom = |gas_limit: u64, witness_len: u64| Claim {
            verifier: Address::repeat_byte(0x7E),
            key: Bytes::from(vec![0x0b; 32]),
            msg: Bytes::from(vec![0x0c; 32]),
            gas_limit,
            witness_len,
        };
        let metadata = 20 * 16 + 32 * 16 + 32 * 16;
        assert_eq!(gas_used(vec![custom(50_000, 0)]) - base, 50_000 + metadata);
        assert_eq!(
            gas_used(vec![custom(50_000, 300)]) - base,
            50_000 + 16 * 300 + metadata
        );
        assert_eq!(gas_used(vec![custom(1, 0)]) - base, 1 + metadata);
    }
}
