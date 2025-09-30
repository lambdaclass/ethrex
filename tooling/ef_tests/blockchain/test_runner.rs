use std::{collections::HashMap, path::Path};

use crate::{
    deserialize::{PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS_REGEX, SENDER_NOT_EOA_REGEX},
    network::Network,
    types::{BlockChainExpectedException, BlockExpectedException, BlockWithRLP, TestUnit},
};
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    error::{ChainError, InvalidBlockError},
    fork_choice::apply_fork_choice,
};
use ethrex_common::{
    constants::EMPTY_KECCACK_HASH,
    types::{
        Account as CoreAccount, Block as CoreBlock, BlockHeader as CoreBlockHeader,
        InvalidBlockHeaderError,
    },
};
use ethrex_prover_lib::backend::Backend;
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::EvmError;
use guest_program::input::ProgramInput;
use regex::Regex;

pub fn parse_and_execute(
    path: &Path,
    skipped_tests: Option<&[&str]>,
    stateless_backend: Option<Backend>,
) -> datatest_stable::Result<()> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tests = parse_tests(path);
    //Test with the Fusaka tests that should pass. TODO: Once we've implemented all the Fusaka EIPs this should be removed
    //EIPs should be added as strings in the format 'eip-XXXX'
    let fusaka_eips_to_test: Vec<&str> = vec![
        "eip-7594", "eip-7883", "eip-7918", "eip-7934", "eip-7892", "eip-7939", "eip-7951",
        "eip-7594", "eip-7825",
    ];

    //Hashes of any other tests to run, that don't correspond to an especific EIP (for examples, some integration tests)
    //We should really remove this once we're finished with implementing Fusaka, but it's a good-enough workaround to run specific tests for now
    let hashes_of_fusaka_tests_to_run: Vec<&str> = vec![
        "0xf0672af9718013a1f396a9268e91e220ff09e7fa97480844e31da500f8ef291f", //All opcodes test
    ];

    // Names of tests to run, to run entire specific .json files. Checked against the TestUnit URl
    let specific_fusaka_tests_to_run: Vec<&str> = vec![
        "/tests/frontier/precompiles/test_precompiles.py",
        "/tests/frontier/precompiles/test_precompile_absence.py",
    ];

    let mut failures = Vec::new();

    for (test_key, test) in tests {
        let test_eip = test.info.clone().reference_spec.unwrap_or_default();

        let should_skip_test = test.network < Network::Merge
            || (test.network > Network::Prague
                && (!fusaka_eips_to_test.iter().any(|eip| test_eip.contains(eip))
                    && !hashes_of_fusaka_tests_to_run
                        .iter()
                        .any(|hash| *hash == test.info.hash.clone().unwrap())
                    && !specific_fusaka_tests_to_run
                        .iter()
                        .any(|name| test.info.url.clone().unwrap().contains(*name))))
            || skipped_tests
                .map(|skipped| skipped.iter().any(|s| test_key.contains(s)))
                .unwrap_or(false);

        if should_skip_test {
            continue;
        }

        let result = rt.block_on(run_ef_test(&test_key, &test, stateless_backend));

        if let Err(e) = result {
            eprintln!("Test {test_key} failed: {e:?}");
            failures.push(format!("{test_key}: {e:?}"));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        // \n doesn't print new lines on terminal, so this alternative is for making it readable
        Err(failures.join("     -------     ").into())
    }
}

pub async fn run_ef_test(
    test_key: &str,
    test: &TestUnit,
    stateless_backend: Option<Backend>,
) -> Result<(), String> {
    // check that the decoded genesis block header matches the deserialized one
    let genesis_rlp = test.genesis_rlp.clone();
    let decoded_block = CoreBlock::decode(&genesis_rlp).unwrap();
    let genesis_block_header = CoreBlockHeader::from(test.genesis_block_header.clone());
    assert_eq!(decoded_block.header, genesis_block_header);

    let store = build_store_for_test(test).await;

    // Check world_state
    check_prestate_against_db(test_key, test, &store);

    // Blockchain EF tests are meant for L1.
    let blockchain = Blockchain::new(store.clone(), BlockchainOptions::default());

    // Early return if the exception is in the rlp decoding of the block
    for bf in &test.blocks {
        if bf.expect_exception.is_some() && exception_in_rlp_decoding(bf) {
            return Ok(());
        }
    }

    run(test_key, test, &blockchain, &store).await?;

    // Run stateless if backend was specified for this.
    // TODO: See if we can run stateless without needing a previous run. We can't easily do it for now. #4142
    if let Some(backend) = stateless_backend {
        re_run_stateless(blockchain, test, test_key, backend).await?;
    };

    Ok(())
}

// Helper: run the EF test blocks and verify poststate
async fn run(
    test_key: &str,
    test: &TestUnit,
    blockchain: &Blockchain,
    store: &Store,
) -> Result<(), String> {
    // Execute all blocks in test
    for block_fixture in test.blocks.iter() {
        let expects_exception = block_fixture.expect_exception.is_some();

        // Won't panic because test has been validated
        let block: CoreBlock = block_fixture.block().unwrap().clone().into();
        let hash = block.hash();

        // Attempt to add the block as the head of the chain
        let chain_result = blockchain.add_block(&block).await;

        match chain_result {
            Err(error) => {
                if !expects_exception {
                    return Err(format!(
                        "Transaction execution unexpectedly failed on test: {test_key}, with error {error:?}",
                    ));
                }
                let expected_exception = block_fixture.expect_exception.clone().unwrap();
                if !exception_is_expected(expected_exception.clone(), &error) {
                    return Err(format!(
                        "Returned exception {error:?} does not match expected {expected_exception:?}",
                    ));
                }
                // Expected exception matched — stop processing further blocks of this test.
                break;
            }
            Ok(_) => {
                if expects_exception {
                    return Err(format!(
                        "Expected transaction execution to fail in test: {test_key} with error: {:?}",
                        block_fixture.expect_exception.clone()
                    ));
                }
                // Advance fork choice to the new head
                apply_fork_choice(store, hash, hash, hash).await.unwrap();
            }
        }
    }

    // Final post-state verification
    check_poststate_against_db(test_key, test, store).await;
    Ok(())
}

fn exception_is_expected(
    expected_exceptions: Vec<BlockChainExpectedException>,
    returned_error: &ChainError,
) -> bool {
    expected_exceptions.iter().any(|exception| {
        if let (
            BlockChainExpectedException::TxtException(expected_error_msg),
            ChainError::EvmError(EvmError::Transaction(error_msg))
            | ChainError::InvalidBlock(InvalidBlockError::InvalidTransaction(error_msg)),
        ) = (exception, returned_error)
        {
            return match_alternative_revm_exception_msg(expected_error_msg, error_msg)
                || (expected_error_msg.to_lowercase() == error_msg.to_lowercase())
                || match_expected_regex(expected_error_msg, error_msg);
        }
        matches!(
            (exception, &returned_error),
            (
                BlockChainExpectedException::BlockException(
                    BlockExpectedException::IncorrectBlobGasUsed
                ),
                ChainError::InvalidBlock(InvalidBlockError::BlobGasUsedMismatch)
            ) | (
                BlockChainExpectedException::BlockException(
                    BlockExpectedException::BlobGasUsedAboveLimit
                ),
                ChainError::InvalidBlock(InvalidBlockError::InvalidHeader(
                    InvalidBlockHeaderError::GasUsedGreaterThanGasLimit
                ))
            ) | (
                BlockChainExpectedException::BlockException(
                    BlockExpectedException::IncorrectExcessBlobGas
                ),
                ChainError::InvalidBlock(InvalidBlockError::InvalidHeader(
                    InvalidBlockHeaderError::ExcessBlobGasIncorrect
                ))
            ) | (
                BlockChainExpectedException::BlockException(
                    BlockExpectedException::IncorrectBlockFormat
                ),
                ChainError::InvalidBlock(_)
            ) | (
                BlockChainExpectedException::BlockException(BlockExpectedException::InvalidRequest),
                ChainError::InvalidBlock(InvalidBlockError::RequestsHashMismatch)
            ) | (
                BlockChainExpectedException::BlockException(
                    BlockExpectedException::SystemContractCallFailed
                ),
                ChainError::EvmError(EvmError::SystemContractCallFailed(_))
            ) | (
                BlockChainExpectedException::Other,
                _ //TODO: Decide whether to support more specific errors.
            ),
        )
    })
}

fn match_alternative_revm_exception_msg(expected_msg: &String, msg: &str) -> bool {
    matches!(
        (msg, expected_msg.as_str()),
        (
            "reject transactions from senders with deployed code",
            SENDER_NOT_EOA_REGEX
        ) | (
            "call gas cost exceeds the gas limit",
            "Transaction gas limit lower than the minimum gas cost to execute the transaction"
        ) | (
            "gas floor exceeds the gas limit",
            "Transaction gas limit lower than the minimum gas cost to execute the transaction"
        ) | ("empty blobs", "Type 3 transaction without blobs")
            | (
                "blob versioned hashes not supported",
                "Type 3 transactions are not supported before the Cancun fork"
            )
            | ("blob version not supported", "Invalid blob versioned hash")
            | (
                "gas price is less than basefee",
                "Insufficient max fee per gas"
            )
            | (
                "blob gas price is greater than max fee per blob gas",
                "Insufficient max fee per blob gas"
            )
            | (
                "priority fee is greater than max fee",
                PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS_REGEX
            )
            | ("create initcode size limit", "Initcode size exceeded")
    ) || (msg.starts_with("lack of funds") && expected_msg == "Insufficient account funds")
}

fn match_expected_regex(expected_error_regex: &str, error_msg: &str) -> bool {
    let Ok(regex) = Regex::new(expected_error_regex) else {
        return false;
    };
    regex.is_match(error_msg)
}

/// Tests the rlp decoding of a block
fn exception_in_rlp_decoding(block_fixture: &BlockWithRLP) -> bool {
    // NOTE: There is a test which validates that an EIP-7702 transaction is not allowed to
    // have the "to" field set to null (create).
    // This test expects an exception to be thrown AFTER the Block RLP decoding, when the
    // transaction is validated. This would imply allowing the "to" field of the
    // EIP-7702 transaction to be null and validating it on the `prepare_execution` LEVM hook.
    //
    // Instead, this approach is taken, which allows for the exception to be thrown on
    // RLPDecoding, so the data type EIP7702Transaction correctly describes the requirement of
    // "to" field to be an Address
    // For more information, please read:
    // - https://eips.ethereum.org/EIPS/eip-7702
    // - https://github.com/lambdaclass/ethrex/pull/2425
    //
    // There is another test which validates the same exact thing, but for an EIP-4844 tx.
    // That test also allows for a "BlockException.RLP_..." error to happen, and that's what is being
    // caught.

    // Decoding_exception_cases = [
    // "BlockException.RLP_",
    // "TransactionException.TYPE_4_TX_CONTRACT_CREATION", ];

    let expects_rlp_exception = block_fixture
        .expect_exception
        .as_ref()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|case| matches!(case, BlockChainExpectedException::RLPException));

    match CoreBlock::decode(block_fixture.rlp.as_ref()) {
        Ok(_) => {
            assert!(!expects_rlp_exception);
            false
        }
        Err(_) => {
            assert!(expects_rlp_exception);
            true
        }
    }
}

pub fn parse_tests(path: &Path) -> HashMap<String, TestUnit> {
    let mut all_tests = HashMap::new();

    if path.is_file() {
        let file_tests = parse_json_file(path);
        all_tests.extend(file_tests);
    } else if path.is_dir() {
        for entry in std::fs::read_dir(path).expect("Failed to read directory") {
            let entry = entry.expect("Failed to get DirEntry");
            let path = entry.path();
            if path.is_dir() {
                let sub_tests = parse_tests(&path); // recursion
                all_tests.extend(sub_tests);
            } else if path.extension().and_then(|s| s.to_str()) == Some("json") {
                let file_tests = parse_json_file(&path);
                all_tests.extend(file_tests);
            }
        }
    } else {
        panic!("Invalid path: not a file or directory");
    }

    all_tests
}

fn parse_json_file(path: &Path) -> HashMap<String, TestUnit> {
    let s = std::fs::read_to_string(path).expect("Unable to read file");
    serde_json::from_str(&s).expect("Unable to parse JSON")
}

/// Creats a new in-memory store and adds the genesis state
pub async fn build_store_for_test(test: &TestUnit) -> Store {
    let store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");
    let genesis = test.get_genesis();
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    store
}

/// Checks db is correct after setting up initial state
/// Panics if any comparison fails
fn check_prestate_against_db(test_key: &str, test: &TestUnit, db: &Store) {
    let block_number = test.genesis_block_header.number.low_u64();
    let db_block_header = db.get_block_header(block_number).unwrap().unwrap();
    let computed_genesis_block_hash = db_block_header.hash();
    // Check genesis block hash
    assert_eq!(test.genesis_block_header.hash, computed_genesis_block_hash);
    // Check genesis state root
    let test_state_root = test.genesis_block_header.state_root;
    assert_eq!(
        test_state_root, db_block_header.state_root,
        "Mismatched genesis state root for database, test: {test_key}"
    );
}

/// Checks that all accounts in the post-state are present and have the correct values in the DB
/// Panics if any comparison fails
/// Tests that previously failed the validation stage shouldn't be executed with this function.
async fn check_poststate_against_db(test_key: &str, test: &TestUnit, db: &Store) {
    let latest_block_number = db.get_latest_block_number().await.unwrap();
    for (addr, account) in &test.post_state {
        let expected_account: CoreAccount = account.clone().into();
        // Check info
        let db_account_info = db
            .get_account_info(latest_block_number, *addr)
            .await
            .expect("Failed to read from DB")
            .unwrap_or_else(|| {
                panic!("Account info for address {addr} not found in DB, test:{test_key}")
            });
        assert_eq!(
            db_account_info, expected_account.info,
            "Mismatched account info for address {addr} test:{test_key}"
        );
        // Check code
        let code_hash = expected_account.info.code_hash;
        if code_hash != *EMPTY_KECCACK_HASH {
            // We don't want to get account code if there's no code.
            let db_account_code = db
                .get_account_code(code_hash)
                .expect("Failed to read from DB")
                .unwrap_or_else(|| {
                    panic!("Account code for code hash {code_hash} not found in DB test:{test_key}")
                });
            assert_eq!(
                db_account_code, expected_account.code,
                "Mismatched account code for code hash {code_hash} test:{test_key}"
            );
        }
        // Check storage
        for (key, value) in expected_account.storage {
            let db_storage_value = db
                .get_storage_at(latest_block_number, *addr, key)
                .await
                .expect("Failed to read from DB")
                .unwrap_or_else(|| {
                    panic!("Storage missing for address {addr} key {key} in DB test:{test_key}")
                });
            assert_eq!(
                db_storage_value, value,
                "Mismatched storage value for address {addr}, key {key} test:{test_key}"
            );
        }
    }
    // Check lastblockhash is in store
    let last_block_number = db.get_latest_block_number().await.unwrap();
    let last_block_hash = db
        .get_block_header(last_block_number)
        .unwrap()
        .unwrap()
        .hash();
    assert_eq!(
        test.lastblockhash, last_block_hash,
        "Last block number does not match"
    );
    // Get block header
    let last_block = db.get_block_header(last_block_number).unwrap();
    assert!(last_block.is_some(), "Block hash is not stored in db");
    // State root was alredy validated by `add_block``
}

async fn re_run_stateless(
    blockchain: Blockchain,
    test: &TestUnit,
    test_key: &str,
    backend: Backend,
) -> Result<(), String> {
    let blocks = test
        .blocks
        .iter()
        .map(|block_fixture| block_fixture.block().unwrap().clone().into())
        .collect::<Vec<CoreBlock>>();

    let test_should_fail = test.blocks.iter().any(|t| t.expect_exception.is_some());

    let witness = blockchain.generate_witness_for_blocks(&blocks).await;
    if test_should_fail && witness.is_err() {
        // We can't generate witness for a test that should fail.
        return Ok(());
    } else if !test_should_fail && witness.is_err() {
        return Err("Failed to create witness for a test that should not fail".into());
    }
    // At this point witness is guaranteed to be Ok
    let execution_witness = witness.unwrap();

    let program_input = ProgramInput {
        blocks,
        execution_witness,
        elasticity_multiplier: ethrex_common::types::ELASTICITY_MULTIPLIER,
        ..Default::default()
    };

    if let Err(e) = ethrex_prover_lib::execute(backend, program_input) {
        if !test_should_fail {
            return Err(format!(
                "Expected test: {test_key} to succeed but failed with {e}"
            ));
        }
    } else if test_should_fail {
        return Err(format!("Expected test: {test_key} to fail but succeeded"));
    }
    Ok(())
}
