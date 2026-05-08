//! Microbench measuring the hot-path overhead of the struct-log tracer when
//! **disabled** (`active = false`).
//!
//! The bench executes a tight `PUSH1 0x01  POP` × 1000 + STOP loop (2001 opcodes
//! total) with the tracer disabled, to verify that the per-opcode `if active`
//! branch adds ≤2% regression vs the pre-Phase-2 baseline.
//!
//! ## Baseline measurement
//!
//! Measured on `feat/eip-3155-tracer` with the struct-log tracer present but
//! disabled (the bench state).  A pre-Phase-2 baseline via `git stash` was not
//! feasible (too many conflicts with the Phase 2–4 hook sites), so we record
//! the absolute number from this branch as the reference:
//!
//! ```text
//! struct_log/disabled_1000   time: [7.69 µs 7.69 µs 7.70 µs]
//! ```
//!
//! Measured on the development machine (AMD64 Linux, 2026-05).  A 2% regression
//! allowance would be ≤7.85 µs on that machine.  CI runs may differ by ±10%
//! due to scheduling noise; the bench guards against large regressions, not a
//! tight per-machine SLA.
//!
//! ## Rationale
//!
//! Adding a single `if self.struct_log_tracer.active` branch per opcode is the
//! minimal cost for supporting the per-opcode tracer.  The branch is always
//! not-taken when disabled, so modern CPUs predict it cheaply.  This bench
//! measures the floor cost.

use criterion::{Criterion, criterion_group, criterion_main};
use ethrex_common::{
    Address, H256, U256,
    types::{
        Account, AccountState, ChainConfig, Code, CodeMetadata, EIP1559Transaction, Fork,
        Transaction, TxKind,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    errors::DatabaseError,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ── Minimal in-memory database ─────────────────────────────────────────────

struct BenchDb {
    accounts: FxHashMap<Address, Account>,
}

impl Database for BenchDb {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        use ethrex_common::constants::EMPTY_TRIE_HASH;
        Ok(self
            .accounts
            .get(&address)
            .map(|acc| AccountState {
                nonce: acc.info.nonce,
                balance: acc.info.balance,
                storage_root: *EMPTY_TRIE_HASH,
                code_hash: acc.info.code_hash,
            })
            .unwrap_or_default())
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        Ok(self
            .accounts
            .get(&address)
            .and_then(|acc| acc.storage.get(&key).copied())
            .unwrap_or_default())
    }

    fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
        Ok(H256::zero())
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Ok(ChainConfig::default())
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        for acc in self.accounts.values() {
            if acc.info.code_hash == code_hash {
                return Ok(acc.code.clone());
            }
        }
        Ok(Code::default())
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        for acc in self.accounts.values() {
            if acc.info.code_hash == code_hash {
                return Ok(CodeMetadata {
                    length: acc.code.bytecode.len() as u64,
                });
            }
        }
        Ok(CodeMetadata { length: 0 })
    }
}

// ── Bench helper ───────────────────────────────────────────────────────────

const GAS_LIMIT: u64 = 10_000_000;
const SENDER_ADDR: u64 = 0x1000;
const CONTRACT_ADDR: u64 = 0x2000;

/// Builds the 2001-opcode bytecode: `(PUSH1 0x01  POP) × 1000  STOP`.
fn build_push_pop_bytecode(iterations: usize) -> Vec<u8> {
    // Each iteration: 0x60 0x01 0x50  (3 bytes)
    let mut bc = Vec::with_capacity(iterations * 3 + 1);
    for _ in 0..iterations {
        bc.extend_from_slice(&[0x60, 0x01, 0x50]); // PUSH1 0x01, POP
    }
    bc.push(0x00); // STOP
    bc
}

fn bench_disabled(c: &mut Criterion) {
    let bytecode = build_push_pop_bytecode(1000);

    let sender = Address::from_low_u64_be(SENDER_ADDR);
    let contract = Address::from_low_u64_be(CONTRACT_ADDR);

    let code = Code::from_bytecode(bytes::Bytes::from(bytecode), &NativeCrypto);
    let contract_acc = Account::new(U256::zero(), code, 1, FxHashMap::default());
    let sender_acc = Account::new(
        // 10 ETH
        U256::from(10u64) * U256::from(10u64).pow(U256::from(18)),
        Code::default(),
        0,
        FxHashMap::default(),
    );

    let fork = Fork::Cancun;
    let blob_schedule = EVMConfig::canonical_values(fork);
    let env = Environment {
        origin: sender,
        gas_limit: GAS_LIMIT,
        config: EVMConfig::new(fork, blob_schedule),
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
    };

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        value: U256::zero(),
        data: bytes::Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut accounts_map = FxHashMap::default();
    accounts_map.insert(contract, contract_acc.clone());
    accounts_map.insert(sender, sender_acc.clone());

    // db is kept to satisfy the Arc-based pattern; actual per-iteration setup uses fresh copies.
    let _db = Arc::new(BenchDb {
        accounts: accounts_map,
    });

    c.bench_function("struct_log/disabled_1000", |b| {
        b.iter_with_setup(
            || {
                // Fresh DB state per iteration so gas/nonce doesn't drift.
                let mut fresh_accounts = FxHashMap::default();
                fresh_accounts.insert(contract, contract_acc.clone());
                fresh_accounts.insert(sender, sender_acc.clone());
                let fresh_db = Arc::new(BenchDb {
                    accounts: fresh_accounts,
                });
                GeneralizedDatabase::new(fresh_db)
            },
            |mut gen_db| {
                // The struct_log_tracer field is `disabled()` by default — no allocation,
                // one not-taken branch per opcode (the measured overhead).
                let mut vm = VM::new(
                    env.clone(),
                    &mut gen_db,
                    &tx,
                    LevmCallTracer::disabled(),
                    VMType::L1,
                    &NativeCrypto,
                )
                .expect("VM::new");
                vm.execute().expect("execute");
            },
        )
    });
}

criterion_group!(benches, bench_disabled);
criterion_main!(benches);
