//! EIP-8037: cost_per_state_byte (CPSB) tests.
//!
//! Per ethereum/EIPs#11573, CPSB is a fixed constant (1174). The original draft
//! defined a dynamic, block-gas-limit-dependent formula with quantization; that
//! mechanism has been removed from the spec, so we only assert the fixed value
//! here.
//!
//! Also covers parity between the standalone `intrinsic_gas_dimensions`
//! helper (used by mempool / payload builder) and `VM::get_intrinsic_gas`
//! (used during actual tx execution). They must agree on every tx shape or
//! mempool admission will drift from VM charge.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        Account, AccountState, AuthorizationTuple, ChainConfig, Code, CodeMetadata,
        EIP1559Transaction, EIP7702Transaction, Fork, Transaction, TxKind,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    errors::DatabaseError,
    gas_cost::cost_per_state_byte,
    tracing::LevmCallTracer,
    utils::intrinsic_gas_dimensions,
    vm::{VM, VMType},
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

/// CPSB is a fixed constant of 1174 (ethereum/EIPs#11573). The block-gas-limit
/// argument is retained in the function signature for forward-compatibility but
/// must not influence the result.
#[test]
fn test_cpsb_is_fixed_at_1174() {
    for gas_limit in [
        1u64,
        5_000_000,
        29_999_999,
        30_000_000,
        96_000_000,
        120_000_000,
        500_000_000,
        u64::MAX,
    ] {
        assert_eq!(
            cost_per_state_byte(gas_limit),
            1174,
            "CPSB must be fixed at 1174 regardless of block gas limit (input: {gas_limit})",
        );
    }
}

// ==================== intrinsic_gas_dimensions parity ====================

struct TestDb;

impl Database for TestDb {
    fn get_account_state(&self, _address: Address) -> Result<AccountState, DatabaseError> {
        Ok(AccountState::default())
    }
    fn get_storage_value(&self, _address: Address, _key: H256) -> Result<U256, DatabaseError> {
        Ok(U256::zero())
    }
    fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
        Ok(H256::zero())
    }
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Ok(ChainConfig::default())
    }
    fn get_account_code(&self, _code_hash: H256) -> Result<Code, DatabaseError> {
        Ok(Code::default())
    }
    fn get_code_metadata(&self, _code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        Ok(CodeMetadata { length: 0 })
    }
}

fn parity_db() -> GeneralizedDatabase {
    let mut accounts: FxHashMap<Address, Account> = FxHashMap::default();
    accounts.insert(
        Address::from_low_u64_be(0x1000),
        Account::new(
            U256::from(10u64).pow(18.into()),
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );
    GeneralizedDatabase::new_with_account_state(Arc::new(TestDb), accounts)
}

fn parity_env(fork: Fork, block_gas_limit: u64) -> Environment {
    let blob_schedule = EVMConfig::canonical_values(fork);
    Environment {
        origin: Address::from_low_u64_be(0x1000),
        gas_limit: 1_000_000,
        config: EVMConfig::new(fork, blob_schedule),
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
        block_gas_limit,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: true,
        is_system_call: false,
    }
}

/// Asserts `intrinsic_gas_dimensions(tx, fork, block_gas_limit)` and
/// `VM::new(env, ...).get_intrinsic_gas()` return the same `(regular, state)`
/// split. A divergence means mempool admission would drift from VM charge.
fn assert_parity(fork: Fork, block_gas_limit: u64, tx: &Transaction) {
    let standalone =
        intrinsic_gas_dimensions(tx, fork, block_gas_limit).expect("intrinsic_gas_dimensions");

    let env = parity_env(fork, block_gas_limit);
    let mut db = parity_db();
    let vm = VM::new(
        env,
        &mut db,
        tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
    )
    .expect("VM::new");
    let from_vm = vm.get_intrinsic_gas().expect("get_intrinsic_gas");

    assert_eq!(
        standalone, from_vm,
        "intrinsic_gas_dimensions and VM::get_intrinsic_gas diverged for fork {fork:?}: \
         standalone={standalone:?}, vm={from_vm:?}"
    );
}

#[test]
fn test_intrinsic_parity_plain_transfer() {
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 1_000_000,
        to: TxKind::Call(Address::from_low_u64_be(0xBEEF)),
        value: U256::from(1u64),
        data: Bytes::new(),
        access_list: Default::default(),
        ..Default::default()
    });
    // Parity across multiple forks to catch fork-gating regressions too.
    for fork in [Fork::Prague, Fork::Osaka, Fork::Amsterdam] {
        assert_parity(fork, 30_000_000, &tx);
        assert_parity(fork, 120_000_000, &tx);
    }
}

#[test]
fn test_intrinsic_parity_create_tx() {
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 1_000_000,
        to: TxKind::Create,
        value: U256::zero(),
        data: Bytes::from(vec![0x60u8, 0x00, 0x60, 0x00, 0xF3]),
        access_list: Default::default(),
        ..Default::default()
    });
    for fork in [Fork::Prague, Fork::Osaka, Fork::Amsterdam] {
        assert_parity(fork, 30_000_000, &tx);
        assert_parity(fork, 120_000_000, &tx);
    }
}

#[test]
fn test_intrinsic_parity_with_calldata_and_access_list() {
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 1_000_000,
        to: TxKind::Call(Address::from_low_u64_be(0xBEEF)),
        value: U256::zero(),
        // Mix zero + non-zero bytes to exercise EIP-2028 weighted calldata
        // AND the EIP-7976 unweighted floor path.
        data: Bytes::from(vec![0u8, 1, 0, 2, 0, 3, 4, 5, 0, 0]),
        access_list: vec![
            (
                Address::from_low_u64_be(0x11),
                vec![H256::from_low_u64_be(1), H256::from_low_u64_be(2)],
            ),
            (
                Address::from_low_u64_be(0x22),
                vec![H256::from_low_u64_be(3)],
            ),
        ],
        ..Default::default()
    });
    for fork in [Fork::Prague, Fork::Osaka, Fork::Amsterdam] {
        assert_parity(fork, 30_000_000, &tx);
        assert_parity(fork, 120_000_000, &tx);
    }
}

#[test]
fn test_intrinsic_parity_eip7702_auth_list() {
    // Dummy authorization tuple — only the count matters for intrinsic gas.
    let auth = AuthorizationTuple {
        chain_id: U256::from(1),
        address: Address::from_low_u64_be(0xAA),
        nonce: 0,
        y_parity: U256::zero(),
        r_signature: U256::from(1),
        s_signature: U256::from(1),
    };
    let tx = Transaction::EIP7702Transaction(EIP7702Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 1_000_000,
        to: Address::from_low_u64_be(0xBEEF),
        value: U256::zero(),
        data: Bytes::new(),
        access_list: Default::default(),
        authorization_list: vec![auth.clone(), auth],
        ..Default::default()
    });
    for fork in [Fork::Prague, Fork::Osaka, Fork::Amsterdam] {
        assert_parity(fork, 30_000_000, &tx);
        assert_parity(fork, 120_000_000, &tx);
    }
}
