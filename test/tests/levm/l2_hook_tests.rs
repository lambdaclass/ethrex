//! Tests for L2 Hook: fee token storage rollback and privileged transaction handling.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{
        Account, AccountState, ChainConfig, Code, CodeMetadata, EIP1559Transaction, Fork,
        PrivilegedL2Transaction, Transaction, TxKind, fee_config::FeeConfig,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    errors::DatabaseError,
    hooks::l2_hook::{
        COMMON_BRIDGE_L2_ADDRESS, FEE_TOKEN_RATIO_ADDRESS, FEE_TOKEN_REGISTRY_ADDRESS,
    },
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ==================== Test Database ====================

struct TestDatabase {
    accounts: FxHashMap<Address, Account>,
}

impl TestDatabase {
    fn new() -> Self {
        Self {
            accounts: FxHashMap::default(),
        }
    }
}

impl Database for TestDatabase {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
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

// ==================== Constants ====================

const SENDER: u64 = 0x1000;
const RECIPIENT: u64 = 0x2000;
const COINBASE: u64 = 0xCCC;

fn eoa(balance: U256) -> Account {
    Account::new(balance, Code::default(), 0, FxHashMap::default())
}

/// Contract that always returns 1 as a 32-byte word.
/// Used for FEE_TOKEN_REGISTRY (isFeeToken→true) and FEE_TOKEN_RATIO (ratio→1).
/// Bytecode: PUSH1 0x01, PUSH1 0x00, MSTORE, PUSH1 0x20, PUSH1 0x00, RETURN
fn returns_one_contract() -> Account {
    Account::new(
        U256::zero(),
        Code::from_bytecode(Bytes::from(vec![
            0x60, 0x01, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
        ])),
        1,
        FxHashMap::default(),
    )
}

/// Fee token contract that modifies storage on every call.
/// Writes 0xBEEF to storage slot 0, then returns 1.
///
/// ```text
/// PUSH2 0xBEEF  PUSH1 0x00  SSTORE       // slot[0] = 0xBEEF
/// PUSH1 0x01    PUSH1 0x00  MSTORE        // mem[0] = 1
/// PUSH1 0x20    PUSH1 0x00  RETURN        // return(0, 32)
/// ```
fn fee_token_sstore_contract(initial_storage: FxHashMap<H256, U256>) -> Account {
    #[rustfmt::skip]
    let bytecode = vec![
        0x61, 0xBE, 0xEF,  // PUSH2 0xBEEF
        0x60, 0x00,         // PUSH1 0x00
        0x55,               // SSTORE
        0x60, 0x01,         // PUSH1 0x01
        0x60, 0x00,         // PUSH1 0x00
        0x52,               // MSTORE
        0x60, 0x20,         // PUSH1 0x20
        0x60, 0x00,         // PUSH1 0x00
        0xf3,               // RETURN
    ];
    Account::new(
        U256::zero(),
        Code::from_bytecode(Bytes::from(bytecode)),
        1,
        initial_storage,
    )
}

/// Regression test for PR #6045 / audit finding: fee token storage rollback.
///
/// When `prepare_execution_fee_token` deducts fees via `lockFee` (which calls
/// `transfer_fee_token`), the fee token contract's storage is modified. If a
/// subsequent validation check fails (here: priority_fee > max_fee_per_gas),
/// `restore_cache_state()` must revert those storage changes.
///
/// Before the fix (PR #6330), `transfer_fee_token` used `vm.db.get_account_mut`
/// directly without backing up storage slots, so `restore_cache_state()` could
/// not revert fee token storage — leaving tokens locked without being paid out.
#[test]
fn fee_token_storage_rolled_back_on_validation_failure() {
    let sender = Address::from_low_u64_be(SENDER);
    let coinbase = Address::from_low_u64_be(COINBASE);
    let fee_token_addr = Address::from_low_u64_be(0xEE00);

    let gas_limit: u64 = 100_000;
    let gas_price = 1000u64;

    // Fee token contract starts with slot 0 = 42.
    // The lockFee call will SSTORE 0xBEEF into slot 0.
    // If rollback works, slot 0 should remain 42 after the failed tx.
    let initial_slot = H256::zero();
    let initial_value = U256::from(42);
    let fee_token_storage: FxHashMap<H256, U256> =
        [(initial_slot, initial_value)].into_iter().collect();

    let accounts: FxHashMap<Address, Account> = [
        // Sender needs ETH for value=0, gas is paid in fee token
        (sender, eoa(U256::zero())),
        (coinbase, eoa(U256::zero())),
        (fee_token_addr, fee_token_sstore_contract(fee_token_storage)),
        (FEE_TOKEN_REGISTRY_ADDRESS, returns_one_contract()),
        (FEE_TOKEN_RATIO_ADDRESS, returns_one_contract()),
        (COMMON_BRIDGE_L2_ADDRESS, eoa(U256::zero())),
    ]
    .into_iter()
    .collect();

    let test_db = TestDatabase::new();
    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(test_db), accounts);

    let fork = Fork::Prague;
    let blob_schedule = EVMConfig::canonical_values(fork);
    let env = Environment {
        origin: sender,
        gas_limit,
        config: EVMConfig::new(fork, blob_schedule),
        block_number: 1,
        coinbase,
        timestamp: 1000,
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        slot_number: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::from(gas_price),
        base_blob_fee_per_gas: U256::from(1),
        gas_price: U256::from(gas_price),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        // priority > max_fee triggers PriorityGreaterThanMaxFeePerGas AFTER fee deduction
        tx_max_priority_fee_per_gas: Some(U256::from(2000)),
        tx_max_fee_per_gas: Some(U256::from(gas_price)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: gas_limit * 2,
        is_privileged: false,
        fee_token: Some(fee_token_addr),
        disable_balance_check: false,
    };

    let fee_config = FeeConfig {
        base_fee_vault: None,
        operator_fee_config: None,
        l1_fee_config: None,
    };

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(Address::from_low_u64_be(0x9999)),
        value: U256::zero(),
        data: Bytes::new(),
        gas_limit,
        max_fee_per_gas: gas_price,
        max_priority_fee_per_gas: 2000, // > max_fee_per_gas → will fail validation
        ..Default::default()
    });

    let mut vm = VM::new(
        env,
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L2(fee_config),
        &NativeCrypto,
    )
    .unwrap();

    // Execute — should fail because priority_fee > max_fee_per_gas
    let result = vm.execute();
    assert!(
        result.is_err(),
        "Expected execute to fail due to PriorityGreaterThanMaxFeePerGas, got: {result:?}"
    );

    // The critical assertion: fee token storage must be rolled back.
    // Before the fix, transfer_fee_token wrote storage without backup,
    // so slot 0 would be 0xBEEF (from the lockFee simulation) instead of 42.
    let fee_token_slot_0 = db
        .get_account(fee_token_addr)
        .unwrap()
        .storage
        .get(&initial_slot)
        .copied()
        .unwrap_or_default();
    assert_eq!(
        fee_token_slot_0, initial_value,
        "Fee token storage slot 0 should be rolled back to {initial_value} after failed validation, \
         but was {fee_token_slot_0} (0xBEEF = {:#x} means rollback failed)",
        U256::from(0xBEEF)
    );
}

/// Privileged tx with intrinsic gas failure must not lose sender funds.
///
/// Scenario: A non-bridge privileged tx has value > 0, sufficient sender balance,
/// but gas_limit < intrinsic gas (21000). The old code debits the sender's balance
/// before validation and then zeroes msg_value on failure, making the refund in
/// finalize_execution a no-op — permanently burning the sender's ETH.
#[test]
fn privileged_tx_intrinsic_gas_failure_preserves_sender_balance() {
    let sender = Address::from_low_u64_be(SENDER);
    let recipient = Address::from_low_u64_be(RECIPIENT);
    let coinbase = Address::from_low_u64_be(COINBASE);

    let initial_balance = U256::from(1_000_000);
    let transfer_value = U256::from(500_000);
    // Gas limit of 100 is well below intrinsic gas (21000 base cost)
    let gas_limit: u64 = 100;

    let test_db = TestDatabase::new();
    let accounts: FxHashMap<Address, Account> = vec![
        (sender, eoa(initial_balance)),
        (recipient, eoa(U256::zero())),
        (coinbase, eoa(U256::zero())),
    ]
    .into_iter()
    .collect();
    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(test_db), accounts);

    let fork = Fork::Prague;
    let blob_schedule = EVMConfig::canonical_values(fork);
    let env = Environment {
        origin: sender,
        gas_limit,
        config: EVMConfig::new(fork, blob_schedule),
        block_number: 1,
        coinbase,
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
        block_gas_limit: gas_limit * 100,
        is_privileged: true,
        fee_token: None,
        disable_balance_check: false,
    };

    let tx = Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 1000,
        max_fee_per_gas: 1000,
        gas_limit,
        to: TxKind::Call(recipient),
        value: transfer_value,
        data: Bytes::new(),
        access_list: vec![],
        from: sender,
        inner_hash: Default::default(),
        sender_cache: Default::default(),
        cached_canonical: Default::default(),
    });

    let fee_config = FeeConfig {
        base_fee_vault: None,
        operator_fee_config: None,
        l1_fee_config: None,
    };

    let mut vm = VM::new(
        env,
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L2(fee_config),
        &NativeCrypto,
    )
    .expect("VM creation should succeed");

    let report = vm
        .execute()
        .expect("Privileged tx execution should not error");

    // The tx should revert (INVALID opcode) because intrinsic gas was too low
    assert!(
        !report.is_success(),
        "Tx should revert due to intrinsic gas failure"
    );

    // The sender's balance must be fully preserved — no funds should be burned.
    let sender_balance_after = db.get_account(sender).unwrap().info.balance;
    assert_eq!(
        sender_balance_after,
        initial_balance,
        "Sender balance must be preserved after failed privileged tx. \
         Expected {initial_balance}, got {sender_balance_after}. \
         Difference (lost funds): {}",
        initial_balance - sender_balance_after
    );

    // The recipient should NOT have received any funds
    let recipient_balance_after = db.get_account(recipient).unwrap().info.balance;
    assert_eq!(
        recipient_balance_after,
        U256::zero(),
        "Recipient should not receive funds from a reverted privileged tx"
    );
}
