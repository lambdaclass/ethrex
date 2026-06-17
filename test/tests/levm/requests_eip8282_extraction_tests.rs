//! EIP-8282 request-extraction tests driven through `extract_all_requests_levm`.
//!
//! The per-contract readers (`read_builder_deposit_requests` /
//! `dequeue_builder_exit_requests`) are `pub(crate)` in `ethrex-vm`, so these
//! tests exercise the public `extract_all_requests_levm` entry point instead.
//! They cover the Amsterdam gate (builder requests appended only on Amsterdam+),
//! the variant/order of the appended entries, the revert/empty-code failure rules
//! that invalidate a block, and output-byte plumbing.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, NativeCrypto, U256,
    constants::EMPTY_TRIE_HASH,
    types::{
        Account, AccountState, BlockHeader, ChainConfig, Code, CodeMetadata, requests::Requests,
    },
};
use ethrex_levm::{
    db::{Database, gen_db::GeneralizedDatabase},
    errors::DatabaseError,
    vm::VMType,
};
use ethrex_vm::EvmError;
use ethrex_vm::backends::levm::extract_all_requests_levm;
use ethrex_vm::system_contracts::{
    BUILDER_DEPOSIT_CONTRACT_ADDRESS, BUILDER_EXIT_CONTRACT_ADDRESS,
    CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS, DEPOSIT_CONTRACT_ADDRESS,
    WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ==================== Test Database ====================

/// In-memory DB for the extraction tests. Unlike the shared `test_db`, this one
/// carries a configurable `ChainConfig` so the Amsterdam fork gate can be toggled.
struct TestDatabase {
    accounts: FxHashMap<Address, Account>,
    chain_config: ChainConfig,
}

impl TestDatabase {
    fn new(chain_config: ChainConfig) -> Self {
        Self {
            accounts: FxHashMap::default(),
            chain_config,
        }
    }

    fn with_account(mut self, address: Address, account: Account) -> Self {
        self.accounts.insert(address, account);
        self
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
        Ok(self.chain_config)
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

// ==================== Helpers ====================

/// Minimal predeploy bytecode that halts successfully and returns empty output:
/// a single STOP opcode (0x00).
const STOP_BYTECODE: [u8; 1] = [0x00];

/// Predeploy bytecode that reverts: PUSH0 PUSH0 REVERT (0x5f5ffd). Unlike empty
/// code (which fails the early empty-code check), this deploys real code that
/// runs and then reverts, exercising the readers' `TxResult::Revert` arm.
const REVERT_BYTECODE: [u8; 3] = [0x5f, 0x5f, 0xfd];

/// Predeploy bytecode that returns a single non-empty byte (0xAB):
/// PUSH1 0xAB, PUSH1 0x00, MSTORE8, PUSH1 0x01, PUSH1 0x00, RETURN.
const RETURN_BYTE_BYTECODE: [u8; 10] = [0x60, 0xAB, 0x60, 0x00, 0x53, 0x60, 0x01, 0x60, 0x00, 0xf3];

fn predeploy(code: Bytes) -> Account {
    Account::new(
        U256::zero(),
        Code::from_bytecode(code, &NativeCrypto),
        0,
        FxHashMap::default(),
    )
}

/// Block header at timestamp 0 so the fork resolves purely from the chain config.
fn header_at_zero() -> BlockHeader {
    BlockHeader {
        number: 1,
        timestamp: 0,
        ..Default::default()
    }
}

/// A chain config with Prague scheduled at 0 and Amsterdam optionally scheduled.
fn chain_config(amsterdam_time: Option<u64>) -> ChainConfig {
    ChainConfig {
        prague_time: Some(0),
        amsterdam_time,
        ..Default::default()
    }
}

/// Allocates every request system-contract predeploy with the given code so the
/// empty-code-failure check passes (when code is non-empty).
fn db_with_all_predeploys(chain_config: ChainConfig, code: Bytes) -> TestDatabase {
    TestDatabase::new(chain_config)
        .with_account(DEPOSIT_CONTRACT_ADDRESS.address, predeploy(code.clone()))
        .with_account(
            WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS.address,
            predeploy(code.clone()),
        )
        .with_account(
            CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS.address,
            predeploy(code.clone()),
        )
        .with_account(
            BUILDER_DEPOSIT_CONTRACT_ADDRESS.address,
            predeploy(code.clone()),
        )
        .with_account(BUILDER_EXIT_CONTRACT_ADDRESS.address, predeploy(code))
}

// ==================== Tests ====================

/// Amsterdam happy path: with all predeploys present and halting successfully,
/// extraction returns 5 entries with builder-deposit (0x03) then builder-exit
/// (0x04) appended after the three pre-Amsterdam requests.
#[test]
fn amsterdam_appends_builder_requests_in_order() {
    let test_db = db_with_all_predeploys(
        chain_config(Some(0)),
        Bytes::copy_from_slice(&STOP_BYTECODE),
    );
    let mut db = GeneralizedDatabase::new(Arc::new(test_db));
    let header = header_at_zero();

    let requests =
        extract_all_requests_levm(&[], &mut db, &header, VMType::L1, &NativeCrypto).expect("ok");

    assert_eq!(
        requests.len(),
        5,
        "Amsterdam extraction must return 5 request entries"
    );
    assert!(
        matches!(requests[0], Requests::Deposit(_)),
        "entry 0 must be Deposit"
    );
    assert!(
        matches!(requests[1], Requests::Withdrawal(_)),
        "entry 1 must be Withdrawal"
    );
    assert!(
        matches!(requests[2], Requests::Consolidation(_)),
        "entry 2 must be Consolidation"
    );
    assert!(
        matches!(requests[3], Requests::BuilderDeposit(_)),
        "entry 3 must be BuilderDeposit (0x03), appended before BuilderExit"
    );
    assert!(
        matches!(requests[4], Requests::BuilderExit(_)),
        "entry 4 must be BuilderExit (0x04), appended last"
    );
}

/// Pre-Amsterdam gate: with Amsterdam unscheduled (Prague-only), extraction
/// returns exactly the three pre-Amsterdam requests and no builder entries.
#[test]
fn pre_amsterdam_returns_only_three_requests() {
    let test_db =
        db_with_all_predeploys(chain_config(None), Bytes::copy_from_slice(&STOP_BYTECODE));
    let mut db = GeneralizedDatabase::new(Arc::new(test_db));
    let header = header_at_zero();

    let requests =
        extract_all_requests_levm(&[], &mut db, &header, VMType::L1, &NativeCrypto).expect("ok");

    assert_eq!(
        requests.len(),
        3,
        "pre-Amsterdam extraction must return exactly 3 request entries (no builder requests)"
    );
    assert!(matches!(requests[0], Requests::Deposit(_)));
    assert!(matches!(requests[1], Requests::Withdrawal(_)));
    assert!(matches!(requests[2], Requests::Consolidation(_)));
}

/// Empty-code failure: an Amsterdam block whose builder-deposit predeploy has no
/// code must invalidate the block (EIP-8282 empty-code-failure rule, mirroring
/// EIP-7002/7251). The other predeploys are present so the failure is isolated.
#[test]
fn empty_builder_deposit_code_invalidates_block() {
    let stop = Bytes::copy_from_slice(&STOP_BYTECODE);
    let test_db = db_with_all_predeploys(chain_config(Some(0)), stop).with_account(
        BUILDER_DEPOSIT_CONTRACT_ADDRESS.address,
        predeploy(Bytes::new()),
    );
    let mut db = GeneralizedDatabase::new(Arc::new(test_db));
    let header = header_at_zero();

    let result = extract_all_requests_levm(&[], &mut db, &header, VMType::L1, &NativeCrypto);

    assert!(
        matches!(result, Err(EvmError::SystemContractCallFailed(_))),
        "empty builder-deposit predeploy code on an Amsterdam block must fail extraction, got: {result:?}"
    );
}

/// An empty builder-exit predeploy (deposit present) also hits the empty-code
/// failure check and invalidates the block.
#[test]
fn empty_builder_exit_code_invalidates_block() {
    let stop = Bytes::copy_from_slice(&STOP_BYTECODE);
    let test_db = db_with_all_predeploys(chain_config(Some(0)), stop).with_account(
        BUILDER_EXIT_CONTRACT_ADDRESS.address,
        predeploy(Bytes::new()),
    );
    let mut db = GeneralizedDatabase::new(Arc::new(test_db));
    let header = header_at_zero();

    let result = extract_all_requests_levm(&[], &mut db, &header, VMType::L1, &NativeCrypto);

    assert!(
        matches!(result, Err(EvmError::SystemContractCallFailed(_))),
        "empty builder-exit predeploy code on an Amsterdam block (deposit present) must fail extraction, got: {result:?}"
    );
}

/// Revert path (distinct from empty-code): a deployed builder-deposit predeploy
/// whose system call reverts must invalidate the block. This exercises the
/// reader's `TxResult::Revert` arm, which the empty-code test cannot reach
/// (that returns before the EVM runs). Asserted via the revert-specific message.
#[test]
fn reverting_builder_deposit_predeploy_invalidates_block() {
    let stop = Bytes::copy_from_slice(&STOP_BYTECODE);
    let test_db = db_with_all_predeploys(chain_config(Some(0)), stop).with_account(
        BUILDER_DEPOSIT_CONTRACT_ADDRESS.address,
        predeploy(Bytes::copy_from_slice(&REVERT_BYTECODE)),
    );
    let mut db = GeneralizedDatabase::new(Arc::new(test_db));
    let header = header_at_zero();

    let result = extract_all_requests_levm(&[], &mut db, &header, VMType::L1, &NativeCrypto);

    match result {
        Err(EvmError::SystemContractCallFailed(msg)) => assert!(
            msg.contains("REVERT when reading builder deposit requests"),
            "expected the builder-deposit revert message, got: {msg}"
        ),
        other => panic!("expected SystemContractCallFailed from the revert arm, got: {other:?}"),
    }
}

/// Builder-exit failure path: the builder-deposit call succeeds (STOP) but the
/// builder-exit call reverts. Confirms the exit reader's error mapping and that
/// extraction reaches the exit call after the deposit call succeeds.
#[test]
fn reverting_builder_exit_predeploy_invalidates_block() {
    let stop = Bytes::copy_from_slice(&STOP_BYTECODE);
    let test_db = db_with_all_predeploys(chain_config(Some(0)), stop).with_account(
        BUILDER_EXIT_CONTRACT_ADDRESS.address,
        predeploy(Bytes::copy_from_slice(&REVERT_BYTECODE)),
    );
    let mut db = GeneralizedDatabase::new(Arc::new(test_db));
    let header = header_at_zero();

    let result = extract_all_requests_levm(&[], &mut db, &header, VMType::L1, &NativeCrypto);

    match result {
        Err(EvmError::SystemContractCallFailed(msg)) => assert!(
            msg.contains("REVERT when dequeuing builder exit requests"),
            "expected the builder-exit revert message, got: {msg}"
        ),
        other => {
            panic!("expected SystemContractCallFailed from the exit revert arm, got: {other:?}")
        }
    }
}

/// Non-empty output plumbing: a predeploy that returns bytes must surface those
/// bytes in the corresponding `Requests` variant (and hence into `requests_hash`).
/// Both builder predeploys return the single byte 0xAB here.
#[test]
fn builder_request_output_bytes_flow_through() {
    let test_db = db_with_all_predeploys(
        chain_config(Some(0)),
        Bytes::copy_from_slice(&RETURN_BYTE_BYTECODE),
    );
    let mut db = GeneralizedDatabase::new(Arc::new(test_db));
    let header = header_at_zero();

    let requests =
        extract_all_requests_levm(&[], &mut db, &header, VMType::L1, &NativeCrypto).expect("ok");

    match &requests[3] {
        Requests::BuilderDeposit(data) => assert_eq!(
            data.as_slice(),
            &[0xAB],
            "builder-deposit request data must carry the system-call output bytes"
        ),
        other => panic!("entry 3 must be BuilderDeposit, got: {other:?}"),
    }
    match &requests[4] {
        Requests::BuilderExit(data) => assert_eq!(
            data.as_slice(),
            &[0xAB],
            "builder-exit request data must carry the system-call output bytes"
        ),
        other => panic!("entry 4 must be BuilderExit, got: {other:?}"),
    }
}
