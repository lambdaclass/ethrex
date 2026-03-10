//! Regression tests for Audit Finding E:
//! Privileged transactions with gas_limit exceeding block_gas_limit
//! must be force-included as reverts, not hard-rejected.

use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        AccountState, ChainConfig, Code, CodeMetadata, PrivilegedL2Transaction, Transaction, TxKind,
    },
};
use ethrex_levm::{
    db::{Database, gen_db::GeneralizedDatabase},
    errors::DatabaseError,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};

/// Minimal in-memory store for testing.
struct TestStore {
    sender: Address,
    sender_balance: U256,
}

impl Database for TestStore {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        if address == self.sender {
            Ok(AccountState {
                balance: self.sender_balance,
                nonce: 0,
                code_hash: H256::zero(),
                storage_root: H256::zero(),
            })
        } else {
            Ok(AccountState::default())
        }
    }

    fn get_storage_value(&self, _: Address, _: H256) -> Result<U256, DatabaseError> {
        Ok(U256::zero())
    }

    fn get_block_hash(&self, _: u64) -> Result<H256, DatabaseError> {
        Ok(H256::zero())
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Err(DatabaseError::Custom("not implemented".into()))
    }

    fn get_account_code(&self, _: H256) -> Result<Code, DatabaseError> {
        Ok(Code::from_bytecode(Bytes::new()))
    }

    fn get_code_metadata(&self, _: H256) -> Result<CodeMetadata, DatabaseError> {
        Ok(CodeMetadata { length: 0 })
    }
}

/// A privileged tx whose gas_limit exceeds the block gas limit must be
/// force-included as a revert (not hard-rejected with GasAllowanceExceeded).
#[test]
fn privileged_tx_with_excessive_gas_limit_force_included_as_revert() {
    let sender = Address::from_low_u64_be(0xA);
    let block_gas_limit: u64 = 30_000_000;

    let store = TestStore {
        sender,
        sender_balance: U256::from(10u64.pow(18)), // 1 ETH
    };
    let mut db = GeneralizedDatabase::new(Arc::new(store));

    // gas_limit = 2x block_gas_limit — exceeds the block limit
    let privileged_tx = PrivilegedL2Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: block_gas_limit * 2,
        to: TxKind::Call(Address::from_low_u64_be(0xB)),
        value: U256::zero(),
        data: Bytes::new(),
        access_list: Vec::new(),
        from: sender,
        inner_hash: Default::default(),
        sender_cache: Default::default(),
        cached_canonical: Default::default(),
    };
    let tx = Transaction::PrivilegedL2Transaction(privileged_tx);

    let env = ethrex_levm::environment::Environment {
        origin: sender,
        gas_limit: block_gas_limit * 2,
        block_gas_limit,
        is_privileged: true,
        block_number: 1,
        ..Default::default()
    };

    let mut vm = VM::new(
        env,
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L2(Default::default()),
    )
    .expect("VM::new should succeed");

    let result = vm.execute();

    // The tx must NOT be hard-rejected — it should return Ok with a failed execution
    let report = result.expect("privileged tx must not be hard-rejected (GasAllowanceExceeded)");
    assert!(
        !report.is_success(),
        "privileged tx with excessive gas should revert, not succeed"
    );
}
