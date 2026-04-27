//! EIP-8037: Dynamic cost_per_state_byte Tests
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

/// Sanity check: cost_per_state_byte(120_000_000) == 1174
/// (matches the legacy hardcoded COST_PER_STATE_BYTE constant)
#[test]
fn test_cpsb_120m() {
    assert_eq!(cost_per_state_byte(120_000_000), 1174);
}

/// gas_limit = 30_000_000
/// num = 30_000_000 * 2_628_000 = 78_840_000_000_000
/// denom = 2 * 100 * 2^30 = 214_748_364_800
/// raw = ceil(78_840_000_000_000 / 214_748_364_800) = 368
/// shifted = 368 + 9578 = 9946
/// bit_length = 14, shift = 9
/// quantized = (9946 >> 9) << 9 = 19 * 512 = 9728
/// result = 9728 - 9578 = 150
#[test]
#[ignore = "bal-devnet-4: cost_per_state_byte temporarily fixed to 1174; re-enable when dynamic formula is restored"]
fn test_cpsb_30m() {
    assert_eq!(cost_per_state_byte(30_000_000), 150);
}

/// gas_limit = 500_000_000
/// raw = ceil(500_000_000 * 2_628_000 / 214_748_364_800) = 6119
/// shifted = 6119 + 9578 = 15697
/// bit_length = 14, shift = 9
/// quantized = (15697 >> 9) << 9 = 30 * 512 = 15360
/// result = 15360 - 9578 = 5782
#[test]
#[ignore = "bal-devnet-4: cost_per_state_byte temporarily fixed to 1174; re-enable when dynamic formula is restored"]
fn test_cpsb_500m() {
    assert_eq!(cost_per_state_byte(500_000_000), 5782);
}

/// Low-end clamp: formula produces `quantized <= CPSB_OFFSET`, so the function
/// returns 1 (the minimum viable cost). Guard against an off-by-one in the
/// `if quantized > CPSB_OFFSET` branch.
#[test]
#[ignore = "bal-devnet-4: cost_per_state_byte temporarily fixed to 1174; re-enable when dynamic formula is restored"]
fn test_cpsb_clamp_to_one_for_tiny_gas_limit() {
    assert_eq!(cost_per_state_byte(1), 1);
    assert_eq!(cost_per_state_byte(5_000_000), 1);
}

/// Upper boundary of the 30M quantization bin — `cpsb(14_999_999)` must not
/// jump across the next bin's value just because `raw` changes by 1. All
/// gas_limits in the 5M–30M range quantize to 150.
#[test]
#[ignore = "bal-devnet-4: cost_per_state_byte temporarily fixed to 1174; re-enable when dynamic formula is restored"]
fn test_cpsb_30m_bin_boundary() {
    assert_eq!(cost_per_state_byte(14_999_999), 150);
    assert_eq!(cost_per_state_byte(15_000_000), 150);
    assert_eq!(cost_per_state_byte(29_999_999), 150);
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
