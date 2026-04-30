use std::{collections::HashMap, path::Path};

use crate::{
    fork::Fork,
    types::{BlockChainExpectedException, BlockExpectedException, BlockWithRLP, TestUnit},
};
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    error::{ChainError, InvalidBlockError},
    fork_choice::apply_fork_choice,
};
#[cfg(feature = "builder-parity")]
use ethrex_blockchain::{
    BlockchainType,
    payload::{BuildPayloadArgs, HeadTransaction, PayloadBuildContext, create_payload},
};
#[cfg(feature = "stateless")]
use ethrex_common::types::block_execution_witness::RpcExecutionWitness;
#[cfg(feature = "builder-parity")]
use ethrex_common::{
    U256,
    types::{ELASTICITY_MULTIPLIER, MempoolTransaction},
};
use ethrex_common::{
    constants::EMPTY_KECCACK_HASH,
    types::{
        Account as CoreAccount, Block as CoreBlock, BlockHeader as CoreBlockHeader,
        InvalidBlockHeaderError, block_access_list::BlockAccessList,
    },
};
#[cfg(feature = "builder-parity")]
use ethrex_crypto::NativeCrypto;
use ethrex_guest_program::input::ProgramInput;
#[cfg(feature = "sp1")]
use ethrex_prover::Sp1Backend;
use ethrex_prover::{BackendType, ExecBackend, ProverBackend};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::EvmError;
use regex::Regex;

pub fn parse_and_execute(
    path: &Path,
    skipped_tests: Option<&[&str]>,
    stateless_backend: Option<BackendType>,
) -> datatest_stable::Result<()> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tests = parse_tests(path);

    let mut failures = Vec::new();

    for (test_key, test) in tests {
        let should_skip_test = test.network < Fork::Merge
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
    stateless_backend: Option<BackendType>,
) -> Result<(), String> {
    // check that the decoded genesis block header matches the deserialized one
    let genesis_rlp = test.genesis_rlp.clone();
    let decoded_block = match CoreBlock::decode(&genesis_rlp) {
        Ok(block) => block,
        Err(e) => return Err(format!("Failed to decode genesis RLP: {e}")),
    };
    let genesis_block_header = CoreBlockHeader::from(test.genesis_block_header.clone());
    if decoded_block.header != genesis_block_header {
        return Err("Decoded genesis header does not match expected header".to_string());
    }

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

    // For Amsterdam tests, exercise the parallel BAL execution path as a correctness check.
    // Two-pass approach: pass 1 collects the BAL produced by sequential execution, pass 2
    // re-executes using that BAL to drive parallel (BAL-warmed) execution and verifies the
    // same final state is reached.
    if test.network == Fork::Amsterdam {
        run_two_pass_parallel(test_key, test).await?;
        #[cfg(feature = "builder-parity")]
        run_builder_parity(test_key, test).await?;
    }

    // Run stateless if backend was specified for this.
    // TODO: See if we can run stateless without needing a previous run. We can't easily do it for now. #4142
    if let Some(backend) = stateless_backend {
        // If the fixture provides an executionWitness (zkevm format), use it directly
        // instead of regenerating the witness from blockchain execution.
        #[cfg(feature = "stateless")]
        {
            let has_fixture_witness = test.blocks.iter().any(|bf| {
                bf.block()
                    .and_then(|b| b.execution_witness.as_ref())
                    .is_some()
            });
            if has_fixture_witness {
                run_stateless_from_fixture(test, test_key, backend).await?;
                return Ok(());
            }
        }
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
        let chain_result = blockchain.add_block_pipeline(block.clone(), None);

        match chain_result {
            Err(error) => {
                if !expects_exception {
                    return Err(format!(
                        "Transaction execution unexpectedly failed on test: {test_key}, with error {error:?}",
                    ));
                }
                let expected_exception = block_fixture.expect_exception.clone().unwrap();
                if !exception_is_expected(expected_exception.clone(), &error) {
                    eprintln!(
                        "Warning: Returned exception {error:?} does not match expected {expected_exception:?}",
                    );
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

/// Two-pass parallel execution check for Amsterdam tests.
///
/// Pass 1 (sequential): runs every block with `add_block_pipeline_bal` to collect the
/// BAL that each block produces.  Pass 2 (parallel): creates a fresh chain and re-runs every
/// block passing the corresponding BAL so the BAL-warmed parallel path is exercised.  The final
/// post-state of pass 2 must match the expected post-state.
async fn run_two_pass_parallel(test_key: &str, test: &TestUnit) -> Result<(), String> {
    // ---- Pass 1: sequential, collect BALs ----
    let store1 = build_store_for_test(test).await;
    let blockchain1 = Blockchain::new(store1.clone(), BlockchainOptions::default());

    let mut bals: Vec<BlockAccessList> = Vec::with_capacity(test.blocks.len());

    for block_fixture in test.blocks.iter() {
        // Skip fixtures that expect an exception — the normal run() already verified them.
        if block_fixture.expect_exception.is_some() {
            return Ok(());
        }

        let block: CoreBlock = block_fixture.block().unwrap().clone().into();
        let hash = block.hash();

        let produced_bal = blockchain1
            .add_block_pipeline_bal(block, None)
            .map_err(|e| format!("Two-pass pass-1 failed for test {test_key}: {e:?}"))?;

        apply_fork_choice(&store1, hash, hash, hash)
            .await
            .map_err(|e| {
                format!("Two-pass pass-1 fork choice failed for test {test_key}: {e:?}")
            })?;

        // If execution produced no BAL (non-Amsterdam block in a transition test), skip pass 2.
        match produced_bal {
            Some(bal) => bals.push(bal),
            None => return Ok(()),
        }
    }

    // ---- Pass 2: parallel (BAL-driven), verify post-state ----
    let store2 = build_store_for_test(test).await;
    let blockchain2 = Blockchain::new(store2.clone(), BlockchainOptions::default());

    for (block_fixture, bal) in test.blocks.iter().zip(bals.iter()) {
        let block: CoreBlock = block_fixture.block().unwrap().clone().into();
        let hash = block.hash();

        blockchain2
            .add_block_pipeline(block, Some(bal))
            .map_err(|e| format!("Two-pass pass-2 (parallel) failed for test {test_key}: {e:?}"))?;

        apply_fork_choice(&store2, hash, hash, hash)
            .await
            .map_err(|e| {
                format!("Two-pass pass-2 fork choice failed for test {test_key}: {e:?}")
            })?;
    }

    // Verify post-state matches expected
    check_poststate_against_db(test_key, test, &store2).await;
    Ok(())
}

/// Drive the block builder over each fixture's transactions and assert it
/// produces a block matching the fixture header. Catches builder/validator
/// drift on EIP-7928 BAL construction and on receipts/state/requests roots,
/// gas accounting, and bloom.
///
/// Skips fixtures with `expect_exception` (validator-side checks) and any
/// fixture containing 4844 blob txs (no blob bundle in the fixture format).
#[cfg(feature = "builder-parity")]
async fn run_builder_parity(test_key: &str, test: &TestUnit) -> Result<(), String> {
    if test.blocks.iter().any(|b| b.expect_exception.is_some()) {
        return Ok(());
    }

    let has_blob_tx = test.blocks.iter().any(|bf| {
        bf.block().is_some_and(|b| {
            b.transactions
                .iter()
                .any(|t| matches!(&t.transaction_type, Some(ty) if ty.low_u64() == 3))
        })
    });
    if has_blob_tx {
        return Ok(());
    }

    let store = build_store_for_test(test).await;
    let blockchain = Blockchain::new(store.clone(), BlockchainOptions::default());

    for block_fixture in test.blocks.iter() {
        let expected: CoreBlock = block_fixture.block().unwrap().clone().into();
        let expected_header = expected.header.clone();

        let args = BuildPayloadArgs {
            parent: expected_header.parent_hash,
            timestamp: expected_header.timestamp,
            fee_recipient: expected_header.coinbase,
            random: expected_header.prev_randao,
            withdrawals: expected.body.withdrawals.clone(),
            beacon_root: expected_header.parent_beacon_block_root,
            slot_number: expected_header.slot_number,
            version: 0,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            gas_ceil: expected_header.gas_limit,
        };

        let payload = create_payload(&args, &store, expected_header.extra_data.clone())
            .map_err(|e| format!("Builder parity {test_key}: create_payload failed: {e:?}"))?;
        let mut ctx = PayloadBuildContext::new(payload, &store, &BlockchainType::L1)
            .map_err(|e| format!("Builder parity {test_key}: ctx failed: {e:?}"))?;

        // calc_gas_limit clamps to parent±delta; force exact match for fixtures
        // that pin a specific gas_limit not reachable by one step from parent.
        ctx.payload.header.gas_limit = expected_header.gas_limit;
        ctx.remaining_gas = expected_header.gas_limit;

        blockchain.apply_system_operations(&mut ctx).map_err(|e| {
            format!("Builder parity {test_key}: apply_system_operations failed: {e:?}")
        })?;

        for tx in &expected.body.transactions {
            let sender = tx
                .sender(&NativeCrypto)
                .map_err(|e| format!("Builder parity {test_key}: sender recovery failed: {e:?}"))?;
            let head = HeadTransaction {
                tx: MempoolTransaction::new(tx.clone(), sender),
                tip: U256::zero(),
            };
            blockchain
                .apply_tx_to_payload(head, &mut ctx)
                .map_err(|e| format!("Builder parity {test_key}: apply_tx failed: {e:?}"))?;
        }

        if ctx.is_amsterdam {
            #[allow(clippy::cast_possible_truncation)]
            let post_tx_index = (ctx.payload.body.transactions.len() + 1) as u16;
            ctx.vm.set_bal_index(post_tx_index);
            if let Some(recorder) = ctx.vm.db.bal_recorder_mut()
                && let Some(withdrawals) = &ctx.payload.body.withdrawals
            {
                recorder.extend_touched_addresses(withdrawals.iter().map(|w| w.address));
            }
        }

        blockchain
            .extract_requests(&mut ctx)
            .map_err(|e| format!("Builder parity {test_key}: extract_requests failed: {e:?}"))?;
        blockchain
            .apply_withdrawals(&mut ctx)
            .map_err(|e| format!("Builder parity {test_key}: apply_withdrawals failed: {e:?}"))?;
        blockchain
            .finalize_payload(&mut ctx)
            .map_err(|e| format!("Builder parity {test_key}: finalize_payload failed: {e:?}"))?;

        let mismatches = collect_header_mismatches(&ctx.payload.header, &expected_header);
        if !mismatches.is_empty() {
            return Err(format!(
                "Builder parity {test_key} block {}: {}",
                expected_header.number,
                mismatches.join("; ")
            ));
        }

        // Advance the chain with the (parity-verified) expected block so the
        // next iteration can use it as parent.
        let hash = expected.hash();
        blockchain
            .add_block_pipeline(expected.clone(), None)
            .map_err(|e| format!("Builder parity {test_key}: add_block failed: {e:?}"))?;
        apply_fork_choice(&store, hash, hash, hash)
            .await
            .map_err(|e| format!("Builder parity {test_key}: fork choice failed: {e:?}"))?;
    }

    Ok(())
}

#[cfg(feature = "builder-parity")]
fn collect_header_mismatches(
    produced: &CoreBlockHeader,
    expected: &CoreBlockHeader,
) -> Vec<String> {
    let mut m = Vec::new();
    if produced.state_root != expected.state_root {
        m.push(format!(
            "state_root: got {} expected {}",
            produced.state_root, expected.state_root
        ));
    }
    if produced.transactions_root != expected.transactions_root {
        m.push(format!(
            "transactions_root: got {} expected {}",
            produced.transactions_root, expected.transactions_root
        ));
    }
    if produced.receipts_root != expected.receipts_root {
        m.push(format!(
            "receipts_root: got {} expected {}",
            produced.receipts_root, expected.receipts_root
        ));
    }
    if produced.withdrawals_root != expected.withdrawals_root {
        m.push(format!(
            "withdrawals_root: got {:?} expected {:?}",
            produced.withdrawals_root, expected.withdrawals_root
        ));
    }
    if produced.requests_hash != expected.requests_hash {
        m.push(format!(
            "requests_hash: got {:?} expected {:?}",
            produced.requests_hash, expected.requests_hash
        ));
    }
    if produced.block_access_list_hash != expected.block_access_list_hash {
        m.push(format!(
            "block_access_list_hash: got {:?} expected {:?}",
            produced.block_access_list_hash, expected.block_access_list_hash
        ));
    }
    if produced.gas_used != expected.gas_used {
        m.push(format!(
            "gas_used: got {} expected {}",
            produced.gas_used, expected.gas_used
        ));
    }
    if produced.logs_bloom != expected.logs_bloom {
        m.push("logs_bloom mismatch".to_string());
    }
    m
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
            return (expected_error_msg.to_lowercase() == error_msg.to_lowercase())
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
                BlockChainExpectedException::BlockException(
                    BlockExpectedException::RlpBlockLimitExceeded
                ),
                ChainError::InvalidBlock(InvalidBlockError::MaximumRlpSizeExceeded(_, _))
            ) | (
                BlockChainExpectedException::Other,
                _ //TODO: Decide whether to support more specific errors.
            ),
        )
    })
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

/// Creates a new in-memory store and adds the genesis state.
pub async fn build_store_for_test(test: &TestUnit) -> Store {
    let mut store =
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
    assert!(db.has_state_root(test_state_root).unwrap());
}

/// Checks that all accounts in the post-state are present and have the correct values in the DB
/// Panics if any comparison fails
/// Tests that previously failed the validation stage shouldn't be executed with this function.
async fn check_poststate_against_db(test_key: &str, test: &TestUnit, db: &Store) {
    let latest_block_number = db.get_latest_block_number().await.unwrap();
    if let Some(post_state) = &test.post_state {
        for (addr, account) in post_state {
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
                        panic!(
                            "Account code for code hash {code_hash} not found in DB test:{test_key}"
                        )
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
    }
    // Check lastblockhash is in store
    let last_block_number = db.get_latest_block_number().await.unwrap();
    let last_block_header = db.get_block_header(last_block_number).unwrap().unwrap();
    let last_block_hash = last_block_header.hash();
    assert_eq!(
        test.lastblockhash, last_block_hash,
        "Last block number does not match"
    );

    // State root was already validated by `add_block`.
}

async fn re_run_stateless(
    blockchain: Blockchain,
    test: &TestUnit,
    test_key: &str,
    backend_type: BackendType,
) -> Result<(), String> {
    let blocks = test
        .blocks
        .iter()
        .map(|block_fixture| block_fixture.block().unwrap().clone().into())
        .collect::<Vec<CoreBlock>>();

    let test_should_fail = test.blocks.iter().any(|t| t.expect_exception.is_some());

    let witness = blockchain.generate_witness_for_blocks(&blocks).await;
    if test_should_fail {
        // The normal run() already verified this test fails correctly.
        // The stateless prover proves valid block execution, not invalid block rejection.
        return Ok(());
    } else if let Err(err) = witness {
        return Err(format!(
            "Failed to create witness for a test that should not fail: {err}"
        ));
    }
    // At this point witness is guaranteed to be Ok
    let execution_witness = witness.unwrap();

    let program_input = ProgramInput::new(blocks, execution_witness);

    let execute_result = match backend_type {
        BackendType::Exec => ExecBackend::new().execute(program_input),
        #[cfg(feature = "sp1")]
        BackendType::SP1 => Sp1Backend::new().execute(program_input),
    };

    if let Err(e) = execute_result {
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

/// Run stateless execution using the execution witness provided directly in the
/// zkevm fixture, instead of generating one from blockchain execution.
///
/// Each block in the fixture has its own `executionWitness` containing the state
/// trie nodes, codes, and ancestor headers needed for that specific block.
/// Following the spec, we execute each block
/// independently with its own witness.
#[cfg(feature = "stateless")]
async fn run_stateless_from_fixture(
    test: &TestUnit,
    test_key: &str,
    backend_type: BackendType,
) -> Result<(), String> {
    let chain_config = test.network.chain_config();

    for block_fixture in test.blocks.iter() {
        // Skip blocks that expect exceptions — those are already validated by the normal path.
        if block_fixture.expect_exception.is_some() {
            continue;
        }

        let Some(block_data) = block_fixture.block() else {
            continue;
        };

        let Some(witness_json) = block_data.execution_witness.as_ref() else {
            continue;
        };

        let block: CoreBlock = block_data.clone().into();
        let block_number = block.header.number;

        let rpc_witness: RpcExecutionWitness = serde_json::from_value(witness_json.clone())
            .map_err(|e| {
                format!("Failed to parse executionWitness for block {block_number}: {e}")
            })?;

        let execution_witness = rpc_witness
            .into_execution_witness(*chain_config, block_number)
            .map_err(|e| format!("Witness conversion failed for block {block_number}: {e}"))?;

        let program_input = ProgramInput::new(vec![block], execution_witness);

        let execute_result = match backend_type {
            BackendType::Exec => ExecBackend::new().execute(program_input),
            #[cfg(feature = "sp1")]
            BackendType::SP1 => Sp1Backend::new().execute(program_input),
        };

        if let Err(e) = execute_result {
            return Err(format!(
                "Stateless execution from fixture failed for {test_key} block {block_number}: {e}"
            ));
        }
    }

    Ok(())
}
