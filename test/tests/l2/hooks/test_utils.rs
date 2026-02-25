//! Test utilities for L2 hook tests.
//!
//! Provides helper functions for creating test environments, databases,
//! transactions, and fee configurations needed to test L2 hooks.

use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::constants::EMPTY_TRIE_HASH;
use ethrex_common::types::fee_config::{FeeConfig, L1FeeConfig, OperatorFeeConfig};
use ethrex_common::types::{
    AccountInfo, AccountState, EIP1559Transaction, Fork, LegacyTransaction, Transaction, TxKind,
};
use ethrex_common::{Address, H160, H256, U256};
use ethrex_levm::EVMConfig;
use ethrex_levm::account::{AccountStatus, LevmAccount};
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::Environment;
use ethrex_levm::errors::{DatabaseError, VMError};
use ethrex_levm::hooks::l2_hook::COMMON_BRIDGE_L2_ADDRESS;
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use once_cell::sync::OnceCell;
use rustc_hash::FxHashMap;

// ============================================================================
// Test Addresses
// ============================================================================

/// Test sender address
pub const TEST_SENDER: Address = H160([
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x01,
]);

/// Test recipient address
pub const TEST_RECIPIENT: Address = H160([
    0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x02,
]);

/// Test coinbase address
pub const TEST_COINBASE: Address = H160([
    0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x03,
]);

/// Test fee token address
pub const TEST_FEE_TOKEN: Address = H160([
    0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x04,
]);

/// Test base fee vault address
pub const TEST_BASE_FEE_VAULT: Address = H160([
    0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x05,
]);

/// Test operator fee vault address
pub const TEST_OPERATOR_VAULT: Address = H160([
    0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x06,
]);

/// Test L1 fee vault address
pub const TEST_L1_FEE_VAULT: Address = H160([
    0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x07,
]);

// ============================================================================
// Default Test Values
// ============================================================================

/// Default gas limit for tests (21000 * 10 = 210000)
pub const DEFAULT_GAS_LIMIT: u64 = 210_000;

/// Default gas price for tests (1 gwei)
pub const DEFAULT_GAS_PRICE: u64 = 1_000_000_000;

/// Default base fee for tests (100 wei)
pub const DEFAULT_BASE_FEE: u64 = 100;

/// Default operator fee per gas (10 wei)
pub const DEFAULT_OPERATOR_FEE_PER_GAS: u64 = 10;

/// Default L1 fee per blob gas (1 wei)
pub const DEFAULT_L1_FEE_PER_BLOB_GAS: u64 = 1;

/// Default sender balance (10 ETH)
pub const DEFAULT_SENDER_BALANCE: u128 = 10_000_000_000_000_000_000;

// ============================================================================
// Environment Creation Helpers
// ============================================================================

/// Creates a test environment configured for L2 execution.
///
/// # Arguments
/// * `is_privileged` - Whether the transaction is privileged (bridge transaction)
/// * `fee_token` - Optional fee token address for ERC20 fee payments
/// * `gas_price` - Effective gas price
/// * `base_fee` - Base fee per gas
#[allow(dead_code)]
pub fn create_test_env_l2(
    is_privileged: bool,
    fee_token: Option<Address>,
    gas_price: U256,
    base_fee: U256,
) -> Environment {
    Environment {
        origin: if is_privileged {
            COMMON_BRIDGE_L2_ADDRESS
        } else {
            TEST_SENDER
        },
        gas_limit: DEFAULT_GAS_LIMIT,
        config: EVMConfig::new(Fork::Cancun, EVMConfig::canonical_values(Fork::Cancun)),
        block_number: U256::from(1),
        coinbase: TEST_COINBASE,
        timestamp: U256::from(1000),
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: base_fee,
        base_blob_fee_per_gas: U256::zero(),
        gas_price,
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: Vec::new(),
        tx_max_priority_fee_per_gas: Some(gas_price.saturating_sub(base_fee)),
        tx_max_fee_per_gas: Some(gas_price),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: u64::MAX,
        is_privileged,
        fee_token,
    }
}

/// Creates a minimal test environment with default values.
#[allow(dead_code)]
pub fn create_minimal_env() -> Environment {
    create_test_env_l2(
        false,
        None,
        U256::from(DEFAULT_GAS_PRICE),
        U256::from(DEFAULT_BASE_FEE),
    )
}

/// Creates a privileged environment (bridge transaction).
#[allow(dead_code)]
pub fn create_privileged_env() -> Environment {
    create_test_env_l2(
        true,
        None,
        U256::from(DEFAULT_GAS_PRICE),
        U256::from(DEFAULT_BASE_FEE),
    )
}

/// Creates an environment for fee token transactions.
#[allow(dead_code)]
pub fn create_fee_token_env(fee_token: Address) -> Environment {
    create_test_env_l2(
        false,
        Some(fee_token),
        U256::from(DEFAULT_GAS_PRICE),
        U256::from(DEFAULT_BASE_FEE),
    )
}

// ============================================================================
// Fee Config Creation Helpers
// ============================================================================

/// Creates a test fee configuration.
///
/// # Arguments
/// * `base_fee_vault` - Optional address to send base fees to
/// * `operator_fee_per_gas` - Optional operator fee per gas (enables operator config if Some)
/// * `l1_fee_per_blob_gas` - Optional L1 fee per blob gas (enables L1 config if Some)
#[allow(dead_code)]
pub fn create_test_fee_config(
    base_fee_vault: Option<Address>,
    operator_fee_per_gas: Option<u64>,
    l1_fee_per_blob_gas: Option<u64>,
) -> FeeConfig {
    FeeConfig {
        base_fee_vault,
        operator_fee_config: operator_fee_per_gas.map(|fee| OperatorFeeConfig {
            operator_fee_vault: TEST_OPERATOR_VAULT,
            operator_fee_per_gas: fee,
        }),
        l1_fee_config: l1_fee_per_blob_gas.map(|fee| L1FeeConfig {
            l1_fee_vault: TEST_L1_FEE_VAULT,
            l1_fee_per_blob_gas: fee,
        }),
    }
}

/// Creates a default fee configuration with all components enabled.
#[allow(dead_code)]
pub fn create_default_fee_config() -> FeeConfig {
    create_test_fee_config(
        Some(TEST_BASE_FEE_VAULT),
        Some(DEFAULT_OPERATOR_FEE_PER_GAS),
        Some(DEFAULT_L1_FEE_PER_BLOB_GAS),
    )
}

/// Creates an empty fee configuration (no vaults, no operator fee, no L1 fee).
#[allow(dead_code)]
pub fn create_empty_fee_config() -> FeeConfig {
    FeeConfig::default()
}

// ============================================================================
// Transaction Creation Helpers
// ============================================================================

/// Creates a simple legacy transaction for testing.
///
/// # Arguments
/// * `to` - Recipient address
/// * `value` - Value to transfer
/// * `gas_limit` - Gas limit
/// * `gas_price` - Gas price
#[allow(dead_code)]
pub fn create_test_tx(to: Address, value: U256, gas_limit: u64, gas_price: U256) -> Transaction {
    Transaction::LegacyTransaction(LegacyTransaction {
        nonce: 0,
        gas_price,
        gas: gas_limit,
        to: TxKind::Call(to),
        value,
        data: Bytes::new(),
        v: U256::zero(),
        r: U256::zero(),
        s: U256::zero(),
        inner_hash: OnceCell::new(),
    })
}

/// Creates a default test transaction.
#[allow(dead_code)]
pub fn create_default_tx() -> Transaction {
    create_test_tx(
        TEST_RECIPIENT,
        U256::zero(),
        DEFAULT_GAS_LIMIT,
        U256::from(DEFAULT_GAS_PRICE),
    )
}

/// Creates a transaction that sends value.
#[allow(dead_code)]
pub fn create_value_transfer_tx(value: U256) -> Transaction {
    create_test_tx(
        TEST_RECIPIENT,
        value,
        DEFAULT_GAS_LIMIT,
        U256::from(DEFAULT_GAS_PRICE),
    )
}

// ============================================================================
// Database Creation Helpers
// ============================================================================

/// A simple in-memory database implementation for testing.
#[derive(Clone)]
pub struct TestDatabase;

impl ethrex_levm::db::Database for TestDatabase {
    fn get_account_state(&self, _address: Address) -> Result<AccountState, DatabaseError> {
        Ok(AccountState::default())
    }

    fn get_storage_value(&self, _address: Address, _key: H256) -> Result<U256, DatabaseError> {
        Ok(U256::zero())
    }

    fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
        Ok(H256::zero())
    }

    fn get_chain_config(&self) -> Result<ethrex_common::types::ChainConfig, DatabaseError> {
        Ok(ethrex_common::types::ChainConfig::default())
    }

    fn get_account_code(
        &self,
        _code_hash: H256,
    ) -> Result<ethrex_common::types::Code, DatabaseError> {
        Ok(ethrex_common::types::Code::default())
    }
}

/// Creates a test database with specified accounts.
///
/// # Arguments
/// * `accounts` - List of (address, balance, nonce) tuples
#[allow(dead_code)]
pub fn create_test_db_with_accounts(accounts: Vec<(Address, U256, u64)>) -> GeneralizedDatabase {
    let store = Arc::new(TestDatabase);
    let mut current_state: FxHashMap<Address, LevmAccount> = FxHashMap::default();

    for (address, balance, nonce) in accounts {
        let account = LevmAccount {
            info: AccountInfo {
                balance,
                nonce,
                code_hash: *EMPTY_TRIE_HASH,
            },
            storage: FxHashMap::default(),
            has_storage: false,
            status: AccountStatus::Unmodified,
        };
        current_state.insert(address, account);
    }

    GeneralizedDatabase {
        store,
        current_accounts_state: current_state.clone(),
        initial_accounts_state: current_state,
        codes: FxHashMap::default(),
        tx_backup: None,
    }
}

/// Creates a test database with a default sender account.
#[allow(dead_code)]
pub fn create_default_test_db() -> GeneralizedDatabase {
    create_test_db_with_accounts(vec![(TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0)])
}

/// Creates a test database with sender and recipient accounts.
#[allow(dead_code)]
pub fn create_test_db_with_recipient() -> GeneralizedDatabase {
    create_test_db_with_accounts(vec![
        (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
        (TEST_RECIPIENT, U256::zero(), 0),
    ])
}

// ============================================================================
// Fee Token Contract Helpers
// ============================================================================

/// Creates a mock fee token registry state that returns true for isFeeToken.
///
/// The registry stores a mapping: token_address -> bool (registered or not)
/// Storage slot is keccak256(token_address . slot_0)
#[allow(dead_code)]
pub fn create_fee_token_registry_storage(
    _registered_tokens: Vec<Address>,
) -> FxHashMap<H256, U256> {
    // For simplicity, we'll create storage that makes the registry return true
    // The actual implementation would need to match the contract's storage layout
    FxHashMap::default()
}

/// Creates a mock fee token ratio storage.
///
/// The ratio storage returns the conversion rate for a fee token.
/// Default ratio is 1 (1:1 conversion)
#[allow(dead_code)]
pub fn create_fee_token_ratio_storage(_token: Address, _ratio: U256) -> FxHashMap<H256, U256> {
    FxHashMap::default()
}

// ============================================================================
// Assertion Helpers
// ============================================================================

/// Asserts that an account balance matches the expected value.
#[allow(dead_code)]
pub fn assert_balance(db: &GeneralizedDatabase, address: Address, expected: U256) {
    let actual = db
        .current_accounts_state
        .get(&address)
        .map(|a| a.info.balance)
        .unwrap_or(U256::zero());
    assert_eq!(
        actual, expected,
        "Balance mismatch for {address:?}: expected {expected}, got {actual}"
    );
}

/// Asserts that an account nonce matches the expected value.
#[allow(dead_code)]
pub fn assert_nonce(db: &GeneralizedDatabase, address: Address, expected: u64) {
    let actual = db
        .current_accounts_state
        .get(&address)
        .map(|a| a.info.nonce)
        .unwrap_or(0);
    assert_eq!(
        actual, expected,
        "Nonce mismatch for {address:?}: expected {expected}, got {actual}"
    );
}

// Re-exports are already imported at the top of the file, no need to re-export.

// ============================================================================
// VM Integration Test Helpers
// ============================================================================

/// Creates a test VM configured for L2 execution.
#[allow(dead_code)]
pub fn create_test_l2_vm<'a>(
    env: &Environment,
    db: &'a mut GeneralizedDatabase,
    tx: &Transaction,
    fee_config: FeeConfig,
) -> Result<VM<'a>, VMError> {
    let vm_type = VMType::L2(fee_config);
    VM::new(env.clone(), db, tx, LevmCallTracer::disabled(), vm_type)
}

/// Creates an EIP-1559 transaction for testing.
#[allow(dead_code)]
pub fn create_eip1559_tx(
    to: Address,
    value: U256,
    gas_limit: u64,
    max_fee_per_gas: u64,
    max_priority_fee_per_gas: u64,
    nonce: u64,
) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        nonce,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        gas_limit,
        to: TxKind::Call(to),
        value,
        data: Bytes::new(),
        access_list: Vec::new(),
        chain_id: 1,
        signature_y_parity: false,
        signature_r: U256::zero(),
        signature_s: U256::zero(),
        inner_hash: OnceCell::new(),
    })
}

/// Creates an environment for EIP-1559 transactions.
#[allow(dead_code)]
pub fn create_eip1559_env(
    origin: Address,
    gas_limit: u64,
    max_fee_per_gas: U256,
    max_priority_fee_per_gas: U256,
    base_fee: U256,
    is_privileged: bool,
) -> Environment {
    Environment {
        origin,
        gas_limit,
        config: EVMConfig::new(Fork::Cancun, EVMConfig::canonical_values(Fork::Cancun)),
        block_number: U256::from(1),
        coinbase: TEST_COINBASE,
        timestamp: U256::from(1000),
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: base_fee,
        base_blob_fee_per_gas: U256::zero(),
        gas_price: max_fee_per_gas,
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: Vec::new(),
        tx_max_priority_fee_per_gas: Some(max_priority_fee_per_gas),
        tx_max_fee_per_gas: Some(max_fee_per_gas),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: u64::MAX,
        is_privileged,
        fee_token: None,
    }
}
