// Temporary helpers to capture struct_log JSON for fixture construction.
// These tests print the full JSON output of each fixture program so we can
// verify gas values and field presence when building hand-derived fixtures.

use super::test_db::TestDatabase;
use bytes::Bytes;
use ethrex_common::{
    Address, U256,
    types::{Account, BlockHeader, Code, EIP1559Transaction, Transaction, TxKind},
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::tracing::StructLogConfig;
use ethrex_levm::vm::VMType;
use ethrex_vm::backends::levm::LEVM;
use once_cell::sync::OnceCell;
use rustc_hash::FxHashMap;
use std::sync::Arc;

fn default_header() -> BlockHeader {
    BlockHeader {
        coinbase: Address::from_low_u64_be(0xCCC),
        base_fee_per_gas: Some(1),
        gas_limit: 30_000_000,
        ..Default::default()
    }
}

fn make_tx(contract: Address, sender: Address) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 10,
        gas_limit: 100_000,
        to: TxKind::Call(contract),
        value: U256::zero(),
        data: Bytes::new(),
        access_list: vec![],
        signature_y_parity: false,
        signature_r: U256::one(),
        signature_s: U256::one(),
        inner_hash: OnceCell::new(),
        sender_cache: {
            let cell = OnceCell::new();
            let _ = cell.set(sender);
            cell
        },
        cached_canonical: OnceCell::new(),
    })
}

fn run_trace(bytecode: Vec<u8>, cfg: StructLogConfig) -> String {
    let contract_addr = Address::from_low_u64_be(0xC000);
    let sender_addr = Address::from_low_u64_be(0x1000);

    let mut accounts = FxHashMap::default();
    accounts.insert(
        contract_addr,
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from(bytecode), &NativeCrypto),
            1,
            FxHashMap::default(),
        ),
    );
    accounts.insert(
        sender_addr,
        Account::new(
            U256::from(10u64) * U256::from(10u64).pow(U256::from(18)),
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );

    let test_db = TestDatabase { accounts };
    let mut db = GeneralizedDatabase::new(Arc::new(test_db));
    let header = default_header();
    let tx = make_tx(contract_addr, sender_addr);

    let result = LEVM::trace_tx_struct_log(&mut db, &header, &tx, cfg, VMType::L1, &NativeCrypto)
        .expect("trace should succeed");

    serde_json::to_string_pretty(&result).expect("serialize")
}

#[test]
#[ignore = "fixture-regen helper: run with `cargo test print_ -- --nocapture --ignored`"]
fn print_sstore_basic_trace() {
    // PUSH1 0x2a PUSH1 0x01 SSTORE STOP
    let bytecode = vec![0x60, 0x2a, 0x60, 0x01, 0x55, 0x00];
    let json = run_trace(bytecode, StructLogConfig::default());
    println!("=== SSTORE BASIC ===\n{}", json);
}

#[test]
#[ignore = "fixture-regen helper: run with `cargo test print_ -- --nocapture --ignored`"]
fn print_mstore_memory_trace() {
    // PUSH1 0x20 PUSH1 0x00 MSTORE STOP
    let bytecode = vec![0x60, 0x20, 0x60, 0x00, 0x52, 0x00];
    let json = run_trace(
        bytecode,
        StructLogConfig {
            enable_memory: true,
            ..Default::default()
        },
    );
    println!("=== MSTORE MEMORY ===\n{}", json);
}

#[test]
#[ignore = "fixture-regen helper: run with `cargo test print_ -- --nocapture --ignored`"]
fn print_identity_return_data_trace() {
    // STATICCALL to identity precompile (0x04) with 1 byte of input
    // Returns input unchanged, demonstrating returnData on the STOP step
    //
    // PUSH1 0x01   60 01   -- store 0x01 in mem[0]
    // PUSH1 0x00   60 00
    // MSTORE8      53
    // PUSH1 0x01   60 01   -- retLen=1
    // PUSH1 0x00   60 00   -- retOffset=0
    // PUSH1 0x01   60 01   -- argsLen=1
    // PUSH1 0x00   60 00   -- argsOffset=0
    // PUSH1 0x04   60 04   -- addr=identity
    // GAS          5a
    // STATICCALL   fa
    // STOP         00
    let bytecode = vec![
        0x60, 0x01, 0x60, 0x00, 0x53, 0x60, 0x01, 0x60, 0x00, 0x60, 0x01, 0x60, 0x00, 0x60, 0x04,
        0x5a, 0xfa, 0x00,
    ];
    let json = run_trace(
        bytecode,
        StructLogConfig {
            enable_return_data: true,
            ..Default::default()
        },
    );
    println!("=== STATICCALL RETURN DATA ===\n{}", json);
}
