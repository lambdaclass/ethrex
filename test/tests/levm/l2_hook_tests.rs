//! Tests for L2 Hook privileged transaction handling.
//!
//! Specifically tests that non-bridge privileged transactions that fail intrinsic
//! gas validation correctly refund the sender's balance.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{
        Account, AccountState, ChainConfig, Code, CodeMetadata, Fork, PrivilegedL2Transaction,
        Transaction, TxKind,
        fee_config::FeeConfig,
    },
};
use ethrex_levm::{
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    errors::DatabaseError,
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
    )
    .expect("VM creation should succeed");

    let report = vm.execute().expect("Privileged tx execution should not error");

    // The tx should revert (INVALID opcode) because intrinsic gas was too low
    assert!(
        !report.is_success(),
        "Tx should revert due to intrinsic gas failure"
    );

    // The sender's balance must be fully preserved — no funds should be burned.
    let sender_balance_after = db.get_account(sender).unwrap().info.balance;
    assert_eq!(
        sender_balance_after, initial_balance,
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
