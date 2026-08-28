//! Deterministic reproduction of bal-devnet-6 fork at canonical block 4108 / tx 84
//!
//! The transaction is a fuzz-generated CREATE that:
//!   - Top: CREATE deploys contract A
//!   - A's init: CREATE2 deploys contract B (success)
//!   - B's init: nested CREATE2 cascade (4 levels deep, all OOG)
//!   - B's init: SELFDESTRUCT(0x0)
//!   - A's init: SELFDESTRUCT(0x0)
//!
//! Canonical (geth/besu) gas_used: 730_509
//! ethrex (current code on bal-devnet-6 branch + EIP-7702 fix): 336_045
//! Δ = -394_464 = exactly 3 × STATE_NEW (= STATE_BYTES_PER_NEW_ACCOUNT × CPSB at gas_limit=200M)
//!
//! Captured from `ethrex-office-3` deployment 2026-05-06 from
//! /tmp/canon_4108.json transactions[84]; init code is the verbatim 487-byte
//! input. block_gas_limit is 200_000_000 to match the devnet's network_params.
//!
//! Goal of this test: reproduce the bug deterministically so we can iterate on
//! the fix without redeploying the devnet.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{
        Account, AccountState, ChainConfig, Code, EIP1559Transaction, Fork, Transaction, TxKind,
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

const BLOCK_GAS_LIMIT: u64 = 200_000_000;
const TX_GAS_LIMIT: u64 = 1_000_000;
const TX_VALUE: u64 = 57_469;
const TX_NONCE: u64 = 423;
const SENDER_HEX: &str = "06bbe8b5246147e1348aec15e44fa59525fa0925";
const CHAIN_ID: u64 = 3151908;

/// 487-byte fuzz-generated init code — verbatim copy of canonical block 4108 tx 84 input
const INIT_CODE_HEX: &str = "7f99a2320aa27f7e7759cda97c81bde7f8a93ea960bbc0182c0992ef38f6b8c4f37f0000000000000000000000000000000000000000000000000000000000062e985b505b405b5b7c75c696ab14a035b837a4f9a767246ee71168188e67742f440167260cce4b603f161a305b5b097069a085595fcf958edf8ef3480aab8c299e3a5b35366202ffff16816202ffff1691508261ffff1692503960cf5f60fcf545ff612853609f607136586786a5253532566a5262a49a5c634a8806ca60ca426330a7ce6f9a867f00000000000000000000000000000000115052b6b1ba144b1f683a948ccd99296000527f215f8936a45cbf9bbad61546415024da23eac0fc2e32b12c3331a4cec350f9356020527f000000000000000000000000000000000117269df619a7597a116db12fba09b46040527f5889c3c8a84e9e71a1112eb4fe24d5d4fea5438fde8ff21f03aee2cccb743b086060527f3d95f33c35ba3d97f8f4002c4e213fa018641b32416f06787162bdc081d363ad60805260806101006100a060006000600c5af1610100516101205161014051610160518161ffff169150836202ffff1693508461ffff169450856202ffff1695508661ffff169650f15b5b732d0dadd17667b1a67976560e1ef552a8e165918b3a6ba4f7f5ee44b378b8801a1ff800";

const CANONICAL_GAS_USED: u64 = 730_509;

struct TestDatabase {
    accounts: FxHashMap<Address, Account>,
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

    fn get_storage_value(&self, _address: Address, _key: H256) -> Result<U256, DatabaseError> {
        Ok(U256::zero())
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
}

fn parse_addr(hex: &str) -> Address {
    let bytes = hex::decode(hex).expect("valid hex address");
    Address::from_slice(&bytes)
}

fn parse_initcode() -> Bytes {
    Bytes::from(hex::decode(INIT_CODE_HEX).expect("valid hex initcode"))
}

#[test]
fn bal_devnet6_block4108_tx84_reproduces_gas_divergence() {
    let sender = parse_addr(SENDER_HEX);
    let initcode = parse_initcode();

    // Sender pre-state: nonce 423, balance large enough to cover gas+value.
    let mut accounts: FxHashMap<Address, Account> = FxHashMap::default();
    accounts.insert(
        sender,
        Account::new(
            U256::from(10_000_000_000_000_000_000u128), // 10 ETH
            Code::default(),
            TX_NONCE,
            FxHashMap::default(),
        ),
    );

    let test_db = TestDatabase {
        accounts: accounts.clone(),
    };
    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(test_db), accounts);

    let fork = Fork::Amsterdam;
    let blob_schedule = EVMConfig::canonical_values(fork);
    let env = Environment {
        origin: sender,
        gas_limit: TX_GAS_LIMIT,
        config: EVMConfig::new(fork, blob_schedule),
        block_number: 4108,
        coinbase: Address::from_low_u64_be(0xCCC),
        timestamp: 1_700_000_000,
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        slot_number: U256::zero(),
        chain_id: U256::from(CHAIN_ID),
        base_fee_per_gas: U256::zero(),
        base_blob_fee_per_gas: U256::from(1),
        gas_price: U256::zero(),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: Some(U256::from(2_000_000_000u64)),
        tx_max_fee_per_gas: Some(U256::from(20_000_000_000u64)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: TX_NONCE,
        block_gas_limit: BLOCK_GAS_LIMIT,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: true,
        is_system_call: false,
    };

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: CHAIN_ID,
        nonce: TX_NONCE,
        max_priority_fee_per_gas: 2_000_000_000,
        max_fee_per_gas: 20_000_000_000,
        gas_limit: TX_GAS_LIMIT,
        to: TxKind::Create,
        value: U256::from(TX_VALUE),
        data: initcode,
        ..Default::default()
    });

    let mut vm = VM::new(
        env,
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
    )
    .unwrap();
    let report = vm.execute().unwrap();

    eprintln!(
        "ethrex report: gas_used={} state_gas_used={} success={}",
        report.gas_used,
        report.state_gas_used,
        report.is_success(),
    );
    eprintln!("canonical (geth/besu): gas_used={CANONICAL_GAS_USED}");
    eprintln!(
        "delta = ethrex - canonical = {}",
        report.gas_used as i64 - CANONICAL_GAS_USED as i64,
    );

    assert!(
        report.is_success(),
        "tx should succeed (canonical status=1), got {:?}",
        report.result
    );
    assert_eq!(
        report.gas_used, CANONICAL_GAS_USED,
        "gas_used divergence: ethrex={}, canonical={}, Δ={:+}",
        report.gas_used,
        CANONICAL_GAS_USED,
        report.gas_used as i64 - CANONICAL_GAS_USED as i64,
    );
}
