use std::{collections::HashMap, path::Path};

use crate::{
    deserialize::{PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS_REGEX, SENDER_NOT_EOA_REGEX},
    network::Network,
    types::{BlockChainExpectedException, BlockExpectedException, BlockWithRLP, TestUnit},
};
use ethrex_blockchain::{
    Blockchain, BlockchainType,
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
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::{EvmEngine, EvmError};
use regex::Regex;
use zkvm_interface::io::ProgramInput;

pub fn parse_and_execute(
    path: &Path,
    evm: EvmEngine,
    skipped_tests: Option<&[&str]>,
) -> datatest_stable::Result<()> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tests = parse_tests(path);

    let mut failures = Vec::new();

    for (test_key, test) in tests {
        let should_skip_test = test.network < Network::Merge
            || skipped_tests
                .map(|skipped| skipped.iter().any(|s| test_key.contains(s)))
                .unwrap_or(false);

        if should_skip_test {
            continue;
        }

        let result = rt.block_on(run_ef_test(&test_key, &test, evm));

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

pub async fn run_ef_test(test_key: &str, test: &TestUnit, evm: EvmEngine) -> Result<(), String> {
    // check that the decoded genesis block header matches the deserialized one
    let genesis_rlp = test.genesis_rlp.clone();
    let decoded_block = CoreBlock::decode(&genesis_rlp).unwrap();
    let genesis_block_header = CoreBlockHeader::from(test.genesis_block_header.clone());
    assert_eq!(decoded_block.header, genesis_block_header);

    let store = build_store_for_test(test).await;

    // Check world_state
    check_prestate_against_db(test_key, test, &store);

    // Blockchain EF tests are meant for L1.
    let blockchain_type = BlockchainType::L1;

    let blockchain = Blockchain::new(evm, store.clone(), blockchain_type);
    // Execute all blocks in test
    for block_fixture in test.blocks.iter() {
        let expects_exception = block_fixture.expect_exception.is_some();
        if exception_in_rlp_decoding(block_fixture) {
            return Ok(());
        }

        // Won't panic because test has been validated
        let block: &CoreBlock = &block_fixture.block().unwrap().clone().into();
        let hash = block.hash();

        // Attempt to add the block as the head of the chain
        let chain_result = blockchain.add_block(block).await;
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
                break;
            }
            Ok(_) => {
                if expects_exception {
                    return Err(format!(
                        "Expected transaction execution to fail in test: {test_key} with error: {:?}",
                        block_fixture.expect_exception.clone()
                    ));
                }
                apply_fork_choice(&store, hash, hash, hash).await.unwrap();
            }
        }
    }
    check_poststate_against_db(test_key, test, &store).await;
    if evm == EvmEngine::LEVM {
        re_run_stateless(blockchain, test, test_key).await;
    }
    Ok(())
}

fn exception_is_expected(
    expected_exceptions: Vec<BlockChainExpectedException>,
    returned_error: &ChainError,
) -> bool {
    expected_exceptions.iter().any(|exception| {
        if let (
            BlockChainExpectedException::TxtException(expected_error_msg),
            ChainError::EvmError(EvmError::Transaction(error_msg)),
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
                    BlockExpectedException::SystemContractEmpty
                ),
                ChainError::EvmError(EvmError::SystemContractEmpty(_))
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
            "Intrinsic gas too low"
        ) | ("gas floor exceeds the gas limit", "Intrinsic gas too low")
            | ("empty blobs", "Type 3 transaction without blobs")
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

async fn re_run_stateless(blockchain: Blockchain, test: &TestUnit, test_key: &str) {
    let blocks = test
        .blocks
        .iter()
        .map(|block_fixture| block_fixture.block().unwrap().clone().into())
        .collect::<Vec<CoreBlock>>();

    let test_should_fail = test.blocks.iter().any(|t| t.expect_exception.is_some());

    let witness = blockchain
        .generate_witness_for_blocks(&blocks)
        .await
        .unwrap_or_else(|_| {
            use ethrex_common::types::block_execution_witness::ExecutionWitnessResult;
            if test_should_fail {
                ExecutionWitnessResult {
                    state_trie_nodes: Some(Vec::new()),
                    storage_trie_nodes: Some(HashMap::new()),
                    ..Default::default()
                }
            } else {
                panic!("Failed to create witness for a test that should not fail")
            }
        });

    let program_input = ProgramInput {
        blocks,
        db: witness,
        elasticity_multiplier: ethrex_common::types::ELASTICITY_MULTIPLIER,
        ..Default::default()
    };

    if let Err(e) = ethrex_prover_lib::execute(program_input) {
        assert!(
            test_should_fail,
            "Expected test: {test_key} to succeed but failed with {e}"
        )
    } else {
        assert!(
            !test_should_fail,
            "Expected test: {test_key} to fail but succeeded"
        )
    }
}
