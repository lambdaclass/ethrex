//! Tests for EIP-8151: Account Code Restricted ecRecover.
//!
//! From Hegota a successful ecrecover pays for an account access and a code read of the recovered
//! address (3100 gas cold, 200 warm, on top of the base 3000), warms it, records it in the block
//! access list, and returns empty output unless the address's raw code is empty or exactly an
//! EIP-7702 delegation designation. A failed recovery still costs 3000 and reads nothing.
//!
//! Each test calls a probe contract that STATICCALLs ecrecover once per gas value it was built
//! with, storing for call `i` the success flag at slot `3i`, the output word at `3i + 1` (left at
//! `SENTINEL` when the call returns nothing) and `RETURNDATASIZE` at `3i + 2`.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{Account, Code, EIP1559Transaction, Fork, Transaction, TxKind},
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    db::gen_db::GeneralizedDatabase,
    environment::{EVMConfig, Environment},
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use rustc_hash::FxHashMap;
use secp256k1::{Message, PublicKey, SECP256K1, SecretKey};
use std::sync::Arc;

use crate::levm::test_db::TestDatabase;

const SENDER: u64 = 0x1000;
const PROBE: u64 = 0x2000;
const GAS_LIMIT: u64 = 1_000_000;
const SENTINEL: u8 = 0xff;

const COLD_CHECKED_RECOVERY: u32 = 6100;
const WARM_CHECKED_RECOVERY: u32 = 3200;
const BASE_RECOVERY: u32 = 3000;

struct Signed {
    signer: Address,
    input: Bytes,
}

/// Signs a fixed digest and returns the 128-byte ecrecover input with its signer.
fn signed_input() -> Signed {
    let secret_key = SecretKey::from_slice(&[0x42; 32]).expect("valid secret key");
    let public_key = PublicKey::from_secret_key(SECP256K1, &secret_key).serialize_uncompressed();
    let signer = Address::from_slice(&keccak(&public_key[1..]).as_bytes()[12..]);

    let digest = keccak(b"EIP-8151");
    let (recovery_id, signature) = SECP256K1
        .sign_ecdsa_recoverable(&Message::from_digest(digest.0), &secret_key)
        .serialize_compact();
    let v = 27 + u8::try_from(i32::from(recovery_id)).expect("recovery id is 0 or 1");

    let mut input = digest.as_bytes().to_vec();
    input.extend_from_slice(H256::from_low_u64_be(v.into()).as_bytes());
    input.extend_from_slice(&signature);
    Signed {
        signer,
        input: Bytes::from(input),
    }
}

/// The probe contract: one ecrecover STATICCALL per entry of `gases`, see the module docs.
fn probe_code(gases: &[u32]) -> Bytes {
    let mut code = vec![0x60, 0x80, 0x60, 0x00, 0x60, 0x00, 0x37]; // CALLDATACOPY(0, 0, 128)
    for (i, gas) in gases.iter().enumerate() {
        let slot = u8::try_from(3 * i).expect("few calls");
        code.extend_from_slice(&[0x60, SENTINEL, 0x60, 0x80, 0x52]); // MSTORE(128, SENTINEL)
        // STATICCALL(gas, 0x01, 0, 128, 128, 32)
        code.extend_from_slice(&[0x60, 0x20, 0x60, 0x80, 0x60, 0x80, 0x60, 0x00, 0x60, 0x01]);
        code.push(0x63); // PUSH4 gas
        code.extend_from_slice(&gas.to_be_bytes());
        code.push(0xfa); // STATICCALL
        code.extend_from_slice(&[0x60, slot, 0x55]); // SSTORE(slot, success)
        code.extend_from_slice(&[0x60, 0x80, 0x51, 0x60, slot + 1, 0x55]); // output word
        code.extend_from_slice(&[0x3d, 0x60, slot + 2, 0x55]); // RETURNDATASIZE
    }
    code.push(0x00); // STOP
    Bytes::from(code)
}

fn env(fork: Fork) -> Environment {
    Environment {
        origin: Address::from_low_u64_be(SENDER),
        gas_limit: GAS_LIMIT,
        config: EVMConfig::new(fork, EVMConfig::canonical_values(fork)),
        block_number: 1,
        coinbase: Address::from_low_u64_be(0xCCC),
        timestamp: 1000,
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        slot_number: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::zero(),
        base_blob_fee_per_gas: U256::from(1),
        gas_price: U256::zero(),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: None,
        tx_max_fee_per_gas: Some(U256::zero()),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: GAS_LIMIT * 2,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: true,
        disable_nonce_check: false,
        disable_gas_allowance_check: false,
        disable_sender_eoa_check: false,
        is_system_call: false,
    }
}

struct Run {
    db: GeneralizedDatabase,
}

impl Run {
    /// Executes the probe with `input` as calldata. `signer_code` is the signer's code (`None`
    /// leaves the account absent); `warm_signer` lists it in the transaction's access list.
    fn new(
        fork: Fork,
        gases: &[u32],
        input: Bytes,
        signer: Address,
        signer_code: Option<Bytes>,
        warm_signer: bool,
    ) -> Self {
        let sender = Address::from_low_u64_be(SENDER);
        let probe = Address::from_low_u64_be(PROBE);
        let mut accounts: FxHashMap<Address, Account> = FxHashMap::default();
        accounts.insert(
            sender,
            Account::new(
                U256::from(10u64).pow(18.into()),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        );
        accounts.insert(
            probe,
            Account::new(
                U256::zero(),
                Code::from_bytecode(probe_code(gases), &NativeCrypto),
                1,
                FxHashMap::default(),
            ),
        );
        if let Some(code) = signer_code {
            accounts.insert(
                signer,
                Account::new(
                    U256::from(1),
                    Code::from_bytecode(code, &NativeCrypto),
                    1,
                    FxHashMap::default(),
                ),
            );
        }
        let mut store = TestDatabase::new();
        store.accounts = accounts.clone();
        let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(store), accounts);
        db.enable_bal_recording();
        db.set_bal_index(1);

        let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            chain_id: 1,
            to: TxKind::Call(probe),
            gas_limit: GAS_LIMIT,
            data: input,
            access_list: if warm_signer {
                vec![(signer, vec![])]
            } else {
                vec![]
            },
            ..Default::default()
        });
        let mut vm = VM::new(
            env(fork),
            &mut db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
            &NativeCrypto,
            None,
        )
        .expect("VM::new");
        let report = vm.execute().expect("execute");
        assert!(
            report.is_success(),
            "probe transaction must succeed: {report:?}"
        );
        Run { db }
    }

    fn slot(&mut self, slot: usize) -> U256 {
        let probe = Address::from_low_u64_be(PROBE);
        self.db
            .get_account(probe)
            .expect("probe account")
            .storage
            .get(&H256::from_low_u64_be(
                u64::try_from(slot).expect("small slot"),
            ))
            .copied()
            .unwrap_or_default()
    }

    /// `(success, output word, returndatasize)` of the `call`-th ecrecover call.
    fn call(&mut self, call: usize) -> (U256, U256, U256) {
        (
            self.slot(3 * call),
            self.slot(3 * call + 1),
            self.slot(3 * call + 2),
        )
    }

    fn in_bal(&mut self, address: Address) -> bool {
        let bal = self.db.take_bal().expect("BAL recording was enabled");
        bal.accounts()
            .iter()
            .any(|account| account.address == address)
    }
}

fn word(address: Address) -> U256 {
    U256::from_big_endian(H256::from(address).as_bytes())
}

fn returned(signer: Address) -> (U256, U256, U256) {
    (U256::one(), word(signer), U256::from(32))
}

fn withheld() -> (U256, U256, U256) {
    (U256::one(), U256::from(SENTINEL), U256::zero())
}

fn out_of_gas() -> (U256, U256, U256) {
    (U256::zero(), U256::from(SENTINEL), U256::zero())
}

fn delegation_to(target: Address) -> Bytes {
    let mut code = vec![0xef, 0x01, 0x00];
    code.extend_from_slice(target.as_bytes());
    Bytes::from(code)
}

#[test]
fn recovery_of_a_cold_signer_costs_6100() {
    let Signed { signer, input } = signed_input();
    let gases = [COLD_CHECKED_RECOVERY - 1, COLD_CHECKED_RECOVERY];
    let mut run = Run::new(Fork::Hegota, &gases, input, signer, None, false);

    assert_eq!(
        run.call(0),
        out_of_gas(),
        "one gas short of the cold code check"
    );
    assert_eq!(
        run.call(1),
        returned(signer),
        "an absent signer has empty code"
    );
}

#[test]
fn recovery_of_a_warm_signer_costs_3200() {
    let Signed { signer, input } = signed_input();
    let gases = [WARM_CHECKED_RECOVERY - 1, WARM_CHECKED_RECOVERY];
    let mut run = Run::new(Fork::Hegota, &gases, input, signer, None, true);

    assert_eq!(
        run.call(0),
        out_of_gas(),
        "one gas short of the warm code check"
    );
    assert_eq!(run.call(1), returned(signer));
}

#[test]
fn only_a_paid_code_check_warms_the_signer() {
    let Signed { signer, input } = signed_input();
    // A failed check leaves the signer cold, so the warm price is not enough after it.
    let gases = [COLD_CHECKED_RECOVERY - 1, WARM_CHECKED_RECOVERY];
    let mut run = Run::new(Fork::Hegota, &gases, input.clone(), signer, None, false);
    assert_eq!(run.call(0), out_of_gas());
    assert_eq!(
        run.call(1),
        out_of_gas(),
        "the failed check must not warm it"
    );
    assert!(
        !run.in_bal(signer),
        "an unpaid code check records no account access"
    );

    // A paid check warms it for the rest of the transaction.
    let gases = [COLD_CHECKED_RECOVERY, WARM_CHECKED_RECOVERY];
    let mut run = Run::new(Fork::Hegota, &gases, input, signer, None, false);
    assert_eq!(run.call(0), returned(signer));
    assert_eq!(run.call(1), returned(signer));
    assert!(run.in_bal(signer), "a paid code check is an account read");
}

#[test]
fn signer_with_code_is_withheld() {
    let Signed { signer, input } = signed_input();
    let code = Bytes::from_static(&[0x00]);
    let mut run = Run::new(
        Fork::Hegota,
        &[COLD_CHECKED_RECOVERY],
        input,
        signer,
        Some(code),
        false,
    );

    assert_eq!(
        run.call(0),
        withheld(),
        "empty output, as for a failed recovery"
    );
    assert!(
        run.in_bal(signer),
        "a rejected check still records the read"
    );
}

#[test]
fn delegated_signer_is_returned() {
    let Signed { signer, input } = signed_input();
    let code = delegation_to(Address::from_low_u64_be(0xdead));
    let mut run = Run::new(
        Fork::Hegota,
        &[COLD_CHECKED_RECOVERY],
        input,
        signer,
        Some(code),
        false,
    );

    assert_eq!(run.call(0), returned(signer));
}

#[test]
fn malformed_delegation_is_withheld() {
    let Signed { signer, input } = signed_input();
    let mut too_long = delegation_to(Address::from_low_u64_be(0xdead)).to_vec();
    too_long.push(0x00);
    let mut run = Run::new(
        Fork::Hegota,
        &[COLD_CHECKED_RECOVERY],
        input,
        signer,
        Some(Bytes::from(too_long)),
        false,
    );

    assert_eq!(run.call(0), withheld());
}

#[test]
fn failed_recovery_costs_3000_and_reads_nothing() {
    let Signed { signer, input } = signed_input();
    let mut bad_v = input.to_vec();
    bad_v[63] = 29;
    let mut run = Run::new(
        Fork::Hegota,
        &[BASE_RECOVERY],
        Bytes::from(bad_v),
        signer,
        None,
        false,
    );

    assert_eq!(run.call(0), withheld());
    assert!(!run.in_bal(signer));
}

#[test]
fn amsterdam_recovery_ignores_signer_code() {
    let Signed { signer, input } = signed_input();
    let code = Bytes::from_static(&[0x00]);
    let mut run = Run::new(
        Fork::Amsterdam,
        &[BASE_RECOVERY],
        input,
        signer,
        Some(code),
        false,
    );

    assert_eq!(run.call(0), returned(signer));
    assert!(!run.in_bal(signer));
}
