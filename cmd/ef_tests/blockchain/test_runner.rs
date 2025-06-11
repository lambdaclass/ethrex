use std::{collections::HashMap, path::Path};

use crate::{
    network::Network,
    types::{BlockWithRLP, TestUnit},
};
use ethrex_blockchain::{fork_choice::apply_fork_choice, Blockchain};
use ethrex_common::types::{
    Account as CoreAccount, Block as CoreBlock, BlockHeader as CoreBlockHeader, EMPTY_KECCACK_HASH,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::EvmEngine;
use zkvm_interface::io::ProgramInput;

pub fn parse_and_execute(path: &Path, evm: EvmEngine, skipped_tests: Option<&[&str]>) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tests = parse_test_file(path);

    for (test_key, test) in tests {
        let should_skip_test = test.network < Network::Merge
            || skipped_tests
                .map(|skipped| skipped.contains(&test_key.as_str()))
                .unwrap_or(false);

        if should_skip_test {
            // Discard this test
            continue;
        }

        rt.block_on(run_ef_test(&test_key, &test, evm));
    }
}

pub async fn run_ef_test(test_key: &str, test: &TestUnit, evm: EvmEngine) {
    // check that the decoded genesis block header matches the deserialized one
    let genesis_rlp = test.genesis_rlp.clone();
    let decoded_block = CoreBlock::decode(&genesis_rlp).unwrap();
    let genesis_block_header = CoreBlockHeader::from(test.genesis_block_header.clone());
    assert_eq!(decoded_block.header, genesis_block_header);

    let store = build_store_for_test(test).await;

    // Check world_state
    check_prestate_against_db(test_key, test, &store);

    let blockchain = Blockchain::new(evm, store.clone());
    // Execute all blocks in test
    for block_fixture in test.blocks.iter() {
        let expects_exception = block_fixture.expect_exception.is_some();
        if exception_in_rlp_decoding(block_fixture) {
            return;
        }

        // Won't panic because test has been validated
        let block: &CoreBlock = &block_fixture.block().unwrap().clone().into();
        let hash = block.hash();

        // Attempt to add the block as the head of the chain
        let chain_result = blockchain.add_block(block).await;
        match chain_result {
            Err(error) => {
                assert!(
                    expects_exception,
                    "Transaction execution unexpectedly failed on test: {}, with error {}",
                    test_key, error
                );
                break;
            }
            Ok(_) => {
                assert!(
                    !expects_exception,
                    "Expected transaction execution to fail in test: {} with error: {}",
                    test_key,
                    block_fixture.expect_exception.clone().unwrap()
                );
                apply_fork_choice(&store, hash, hash, hash).await.unwrap();
            }
        }
    }
    check_poststate_against_db(test_key, test, &store).await;
    if evm == EvmEngine::LEVM {
        re_run_stateless(blockchain, test, test_key).await;
    }
}

/// Tests the rlp decoding of a block
fn exception_in_rlp_decoding(block_fixture: &BlockWithRLP) -> bool {
    let decoding_exception_cases = [
        "BlockException.RLP_",
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
        "TransactionException.TYPE_4_TX_CONTRACT_CREATION",
    ];

    let expects_rlp_exception = decoding_exception_cases.iter().any(|&case| {
        block_fixture
            .expect_exception
            .as_ref()
            .map_or(false, |s| s.starts_with(case))
    });

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

pub fn parse_test_file(path: &Path) -> HashMap<String, TestUnit> {
    let s: String = std::fs::read_to_string(path).expect("Unable to read file");
    let tests: HashMap<String, TestUnit> = serde_json::from_str(&s).expect("Unable to parse JSON");
    tests
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
