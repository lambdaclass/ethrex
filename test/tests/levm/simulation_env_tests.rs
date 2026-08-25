//! eth_call / eth_estimateGas environment construction.
//!
//! Simulation must never be stricter than execution: it predicts what
//! execution does, so a header the executor would run with `SLOTNUM` reading
//! zero must not make `eth_call` fail. The simulation env builder used to
//! reject a header whose `slot_number` was `None` under Amsterdam+ rules while
//! the execution env builder defaulted it to zero; these tests pin the two to
//! the same behavior.
//!
//! A canonical Amsterdam header always carries a slot — header validation
//! rejects one that doesn't, the genesis builder fills in `Some(0)`, and
//! `engine_newPayloadV5` requires the field — so the divergence is not
//! reachable through normal block import. It is reachable when a devnet's fork
//! timestamps are moved so that Amsterdam retroactively covers headers already
//! stored under earlier rules.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{Account, BlockHeader, ChainConfig, Code, GenericTransaction};
use ethrex_common::{Address, U256, constants::EMPTY_TRIE_HASH};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::vm::VMType;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::backends::levm::LEVM;
use ethrex_vm::{DynVmDatabase, ExecutionResult};
use rustc_hash::FxHashMap;
use std::sync::Arc;

const SENDER: Address = Address::repeat_byte(0xAA);
/// Holds `SLOTNUM_READER_CODE`, so calling it returns the slot the environment
/// was built with. Asserting on that value is what distinguishes a slot that
/// actually reached the environment from one that was silently dropped.
const SLOTNUM_READER: Address = Address::repeat_byte(0xBB);

/// Holds `GASLIMIT_READER_CODE`, so calling it returns the block gas limit the
/// environment was built with.
const GASLIMIT_READER: Address = Address::repeat_byte(0xCC);

/// `SLOTNUM; PUSH0; MSTORE; PUSH1 0x20; PUSH0; RETURN` — returns the slot
/// number as a 32-byte word.
const SLOTNUM_READER_CODE: [u8; 7] = [0x4B, 0x5F, 0x52, 0x60, 0x20, 0x5F, 0xF3];

/// `GASLIMIT; PUSH1 0x00; MSTORE; PUSH1 0x20; PUSH1 0x00; RETURN` — returns the
/// block gas limit as a 32-byte word. Byte-for-byte the probe used to report the
/// divergence against the other five clients (`0x4560005260206000f3`).
const GASLIMIT_READER_CODE: [u8; 9] = [0x45, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xF3];

/// The header gas limit every test in this file builds its block with.
const BLOCK_GAS_LIMIT: u64 = 30_000_000;

fn amsterdam_db_and_header(slot_number: Option<u64>) -> (GeneralizedDatabase, BlockHeader) {
    let mut store = Store::new("", EngineType::InMemory).unwrap();
    // Activate everything up to Amsterdam from genesis.
    let chain_config = ChainConfig {
        shanghai_time: Some(0),
        cancun_time: Some(0),
        prague_time: Some(0),
        osaka_time: Some(0),
        amsterdam_time: Some(0),
        ..Default::default()
    };
    // Set the config on the SAME store the VM reads (the field is a per-Store
    // in-memory copy, so setting it on a clone would not propagate).
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        store.set_chain_config(&chain_config).await.unwrap();
    });

    let header = BlockHeader {
        number: 1,
        timestamp: 1,
        gas_limit: BLOCK_GAS_LIMIT,
        base_fee_per_gas: Some(7),
        state_root: *EMPTY_TRIE_HASH,
        slot_number,
        ..Default::default()
    };

    let vm_db: DynVmDatabase = Box::new(StoreVmDatabase::new(store, header.clone()).unwrap());
    let mut cache: FxHashMap<Address, Account> = FxHashMap::default();
    cache.insert(
        SENDER,
        Account::new(
            U256::from(10u64).pow(U256::from(18u64)),
            Code::from_bytecode(Bytes::new(), &NativeCrypto),
            0,
            FxHashMap::default(),
        ),
    );
    cache.insert(
        SLOTNUM_READER,
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from_static(&SLOTNUM_READER_CODE), &NativeCrypto),
            0,
            FxHashMap::default(),
        ),
    );
    cache.insert(
        GASLIMIT_READER,
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from_static(&GASLIMIT_READER_CODE), &NativeCrypto),
            0,
            FxHashMap::default(),
        ),
    );
    (
        GeneralizedDatabase::new_with_account_state(Arc::new(vm_db), cache),
        header,
    )
}

fn read_slot_number() -> GenericTransaction {
    GenericTransaction {
        from: SENDER,
        to: ethrex_common::types::TxKind::Call(SLOTNUM_READER),
        ..Default::default()
    }
}

/// Simulates a call to `SLOTNUM_READER` on an Amsterdam header carrying
/// `slot_number` and returns the slot the environment ended up with.
fn simulated_slot_number(slot_number: Option<u64>) -> U256 {
    let (mut db, header) = amsterdam_db_and_header(slot_number);
    let result = LEVM::simulate_tx_from_generic(
        &read_slot_number(),
        &header,
        &mut db,
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("simulation must not error");
    match result {
        ExecutionResult::Success { output, .. } => U256::from_big_endian(&output),
        other => panic!("expected SLOTNUM call to succeed, got {other:?}"),
    }
}

#[test]
fn simulation_tolerates_missing_slot_number_on_amsterdam() {
    // This used to fail with "slot_number must be present in Amsterdam+
    // blocks" while the execution env builder defaulted the same header's slot
    // to zero and ran the transaction. Asserting on the value read by SLOTNUM
    // pins simulation to that same zero default, not merely to "not an error".
    assert_eq!(
        simulated_slot_number(None),
        U256::zero(),
        "a missing slot_number must reach SLOTNUM as zero, like execution does"
    );
}

#[test]
fn simulation_uses_the_slot_number_when_present() {
    assert_eq!(
        simulated_slot_number(Some(1234)),
        U256::from(1234u64),
        "SLOTNUM must read the header's slot_number"
    );
}

/// Simulates a call to `GASLIMIT_READER` and returns the block gas limit the
/// environment ended up with. `gas` is the call object's own gas field, left
/// unset when `None`.
fn simulated_block_gas_limit(gas: Option<u64>) -> U256 {
    let (mut db, header) = amsterdam_db_and_header(Some(0));
    let tx = GenericTransaction {
        from: SENDER,
        to: ethrex_common::types::TxKind::Call(GASLIMIT_READER),
        gas,
        ..Default::default()
    };
    let result = LEVM::simulate_tx_from_generic(&tx, &header, &mut db, VMType::L1, &NativeCrypto, None)
        .expect("simulation must not error");
    match result {
        ExecutionResult::Success { output, .. } => U256::from_big_endian(&output),
        other => panic!("expected GASLIMIT call to succeed, got {other:?}"),
    }
}

#[test]
fn simulation_reports_the_header_gas_limit_to_gaslimit() {
    // The simulation env used to set `block_gas_limit` to `i64::MAX` in order to
    // skip the block gas-allowance check. That field is what opcode 0x45 pushes,
    // so eth_call reported 2^63-1 where the other five clients reported the
    // header's limit, and any contract budgeting against GASLIMIT read nonsense
    // under simulation. The bypass now has its own flag.
    assert_eq!(
        simulated_block_gas_limit(Some(0x200000)),
        U256::from(BLOCK_GAS_LIMIT),
        "GASLIMIT under eth_call must equal the header's gas limit"
    );
}

#[test]
fn simulation_still_runs_a_call_asking_for_more_gas_than_the_block_allows() {
    // The other half of the contract: keeping `block_gas_limit` honest must not
    // reintroduce the gas-allowance rejection that the `i64::MAX` hack was there
    // to avoid. A call may ask for more gas than a block could ever hold.
    assert_eq!(
        simulated_block_gas_limit(Some(BLOCK_GAS_LIMIT * 2)),
        U256::from(BLOCK_GAS_LIMIT),
        "an over-limit `gas` must still run, and still see the real GASLIMIT"
    );
}
