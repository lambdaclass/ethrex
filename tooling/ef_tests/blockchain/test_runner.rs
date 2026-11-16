use std::{collections::HashMap, path::Path, sync::Arc};

use crate::{
    fork::Fork,
    types::{BlockChainExpectedException, BlockExpectedException, BlockWithRLP, TestUnit},
};
use ethrex_blockchain::{
    Blockchain,
    error::{ChainError, InvalidBlockError},
    fork_choice::apply_fork_choice,
};
use ethrex_common::types::block_access_list::{AccountChanges, BlockAccessList, SlotChange};
use ethrex_common::types::block_execution_witness::ExecutionWitness;
#[cfg(feature = "stateless")]
use ethrex_common::types::block_execution_witness::RpcExecutionWitness;
use ethrex_common::{
    H256,
    constants::EMPTY_KECCAK_HASH,
    types::{
        Account as CoreAccount, Block as CoreBlock, BlockHeader as CoreBlockHeader,
        InvalidBlockHeaderError,
    },
    utils::keccak,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::structs::Encoder;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::EvmError;
use regex::Regex;

thread_local! {
    /// Per-OS-thread merkleization pool, lazily built on first use. Mirrors the
    /// pattern used by `tooling/ef_tests/engine` so the ~10k+ blockchain tests
    /// don't each spawn a fresh 17-thread rayon pool of their own.
    /// The merkle protocol's 16 worker jobs cross-communicate via channels, so
    /// each pool may have only one concurrent `in_place_scope` caller; keying by
    /// `thread_local!` makes the calling test-runner thread the natural
    /// exclusive owner.
    static MERKLE_POOL: std::cell::OnceCell<Arc<rayon::ThreadPool>> =
        const { std::cell::OnceCell::new() };
}

fn merkle_pool() -> Arc<rayon::ThreadPool> {
    MERKLE_POOL.with(|cell| cell.get_or_init(Blockchain::build_merkle_pool).clone())
}

/// Tests skipped by every run: they take too long, or ethrex cannot represent them.
pub const SKIPPED_TESTS: &[&str] = &[
    // Skip because they take too long to run, but they pass
    "static_Call50000_sha256",
    "CALLBlake2f_MaxRounds",
    "loopMul",
    // Skip because it tries to deserialize number > U256::MAX
    "ValueOverflowParis",
    // Skip because it's a "Create" Blob Transaction, which doesn't actually exist. It never reaches the EVM because we can't even parse it as an actual Transaction.
    "createBlobhashTx",
];

/// How a fixture's blocks are executed.
#[derive(Debug, Clone, Copy)]
pub struct RunOptions {
    /// Run blocks that carry an access list on the BAL-driven parallel executor,
    /// as a node does unless started with `--no-bal-parallel-exec`.
    pub bal_parallel_exec: bool,
    /// Import every block without the access list its fixture delivers, so the client
    /// executes it without one and checks the list it builds against the header.
    pub withhold_access_lists: bool,
    /// Cache precompile results within a block, as a node does unless started with
    /// `--no-precompile-cache`.
    pub precompile_cache: bool,
    /// Check ethrex's errors against the fixture's expected exceptions, as the
    /// ef_tests suite does: a block that fails to decode must expect a decoding
    /// exception, and an import error other than the expected one is warned about.
    /// Without it a fixture's verdict depends only on which blocks it expects to be
    /// rejected.
    pub check_exception_names: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            bal_parallel_exec: true,
            withhold_access_lists: false,
            precompile_cache: true,
            check_exception_names: true,
        }
    }
}

/// The `Blockchain` a fixture's blocks are imported with, set up as `options` say.
pub fn fixture_blockchain(store: Store, options: &RunOptions) -> Blockchain {
    let mut blockchain = Blockchain::for_test_harness_with_pool(store, merkle_pool());
    blockchain.options.bal_parallel_exec_enabled = options.bal_parallel_exec;
    blockchain.options.precompile_cache_enabled = options.precompile_cache;
    blockchain
}

/// A block the client rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    /// The block's position in the fixture's `blocks`, from 0.
    pub index: usize,
    /// The block's hash, unless the block did not decode.
    pub hash: Option<H256>,
    /// The client's error, as it reports it.
    pub error: String,
}

/// Why a fixture is not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// Its name contains an entry of the skip list.
    Named,
    /// Its fork is before the Merge, which ethrex does not execute.
    PreMerge,
}

pub fn skip_reason(
    test_key: &str,
    test: &TestUnit,
    skipped_tests: Option<&[&str]>,
) -> Option<SkipReason> {
    if skipped_tests.is_some_and(|skipped| skipped.iter().any(|s| test_key.contains(s))) {
        return Some(SkipReason::Named);
    }
    if test.network < Fork::Merge {
        return Some(SkipReason::PreMerge);
    }
    None
}

pub fn parse_and_execute(
    path: &Path,
    skipped_tests: Option<&[&str]>,
    run_stateless: bool,
) -> datatest_stable::Result<()> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tests = parse_tests(path);

    let mut failures = Vec::new();
    let mut executed = 0usize;
    let mut skipped_by_name = 0usize;
    let parsed = tests.len();

    for (test_key, test) in tests {
        match skip_reason(&test_key, &test, skipped_tests) {
            Some(SkipReason::Named) => {
                skipped_by_name += 1;
                continue;
            }
            Some(SkipReason::PreMerge) => continue,
            None => {}
        }
        executed += 1;

        let result = rt.block_on(run_ef_test(
            &test_key,
            &test,
            run_stateless,
            &RunOptions::default(),
            &mut Vec::new(),
        ));

        if let Err(e) = result {
            eprintln!("Test {test_key} failed: {e:?}");
            failures.push(format!("{test_key}: {e:?}"));
        }
    }

    // A stateless fixture file that runs nothing, and was not deliberately named
    // in the skip list, is a failure rather than a pass. A blanket fork-based skip
    // once silenced the entire stateless run — 23,946 fixtures, all of them
    // `network: Amsterdam` — and it stayed green the whole time. Named skips are
    // exempt because they are a recorded decision; a structural skip is not. Not
    // applied to the levm run, where `vectors/legacy/` is legitimately pre-Merge.
    if run_stateless && parsed > 0 && executed == 0 && skipped_by_name == 0 {
        return Err(format!(
            "{}: all {parsed} stateless fixture(s) were skipped structurally, so this file \
             tested nothing",
            path.display()
        )
        .into());
    }

    if failures.is_empty() {
        Ok(())
    } else {
        // \n doesn't print new lines on terminal, so this alternative is for making it readable
        Err(failures.join("     -------     ").into())
    }
}

/// Run one fixture, adding each block the client rejects to `rejections` as it
/// happens, so they are there even when the fixture then fails or panics.
pub async fn run_ef_test(
    test_key: &str,
    test: &TestUnit,
    run_stateless: bool,
    options: &RunOptions,
    rejections: &mut Vec<Rejection>,
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
    let blockchain = fixture_blockchain(store.clone(), options);

    // Early return if the exception is in the rlp decoding of the block
    if options.check_exception_names {
        for (index, bf) in test.blocks.iter().enumerate() {
            if bf.expect_exception.is_none() {
                continue;
            }
            if let Err(error) = CoreBlock::decode(bf.rlp.as_ref()) {
                rejections.push(Rejection {
                    index,
                    hash: None,
                    error: error.to_string(),
                });
            }
            if exception_in_rlp_decoding(bf) {
                return Ok(());
            }
        }
    }

    run(test_key, test, &blockchain, &store, options, rejections).await?;

    // For Amsterdam tests, exercise the parallel BAL execution path as a correctness check.
    // Two-pass approach: pass 1 collects the BAL produced by sequential execution, pass 2
    // re-executes using that BAL to drive parallel (BAL-warmed) execution and verifies the
    // same final state is reached.
    // Not exercised under `stateless`: the stateless harness runs the guest program directly
    // and doesn't drive `add_block_pipeline`, and BAL-warmed parallel execution gives no
    // benefit in single-threaded zkVM guest builds. The non-stateless runs are the right
    // home for this check.
    // Only needed when the fixture delivers no access lists: otherwise `run` has
    // already executed every block in parallel on the fixture's own lists. Skipped
    // when parallel execution is off, since pass 2 would run it anyway.
    #[cfg(not(feature = "stateless"))]
    if test.network == Fork::Amsterdam && options.bal_parallel_exec && !delivers_access_lists(test)
    {
        run_two_pass_parallel(test_key, test, options).await?;
    }

    // Run stateless if backend was specified for this.
    // TODO: See if we can run stateless without needing a previous run. We can't easily do it for now. #4142
    // The `stateless_backend` option now only selects *whether* to run stateless
    // validation, not which backend: the in-memory paths go through
    // `validate_blocks_statelessly` and the wire path through the guest
    // entrypoint, neither of which is backend-dispatched.
    if run_stateless {
        // Use the fixture's witness when present (either `executionWitness` or
        // `statelessInputBytes`); otherwise regenerate by re-running execution.
        #[cfg(feature = "stateless")]
        {
            let has_fixture_witness = test.blocks.iter().any(|bf| {
                bf.block().is_some_and(|b| {
                    b.execution_witness.is_some() || b.stateless_input_bytes.is_some()
                })
            });
            if has_fixture_witness {
                run_stateless_from_fixture(test, test_key).await?;
                check_witness_generation_against_fixture(&blockchain, test, test_key).await?;
                return Ok(());
            }
        }
        re_run_stateless(blockchain, test, test_key).await?;
    };

    Ok(())
}

// Helper: run the EF test blocks and verify poststate
async fn run(
    test_key: &str,
    test: &TestUnit,
    blockchain: &Blockchain,
    store: &Store,
    options: &RunOptions,
    rejections: &mut Vec<Rejection>,
) -> Result<(), String> {
    // Execute all blocks in test
    for (index, block_fixture) in test.blocks.iter().enumerate() {
        let expects_exception = block_fixture.expect_exception.is_some();

        // A block that does not decode is rejected before import, and has no hash.
        if let Err(error) = CoreBlock::decode(block_fixture.rlp.as_ref()) {
            rejections.push(Rejection {
                index,
                hash: None,
                error: error.to_string(),
            });
            if !expects_exception {
                return Err(format!(
                    "Block {index} unexpectedly failed to decode on test: {test_key}, with error {error:?}",
                ));
            }
            continue;
        }

        // Won't panic because test has been validated
        let block: CoreBlock = block_fixture.block().unwrap().clone().into();
        let hash = block.hash();
        let bal = if options.withhold_access_lists {
            if block_fixture
                .block()
                .is_some_and(|block| block.block_access_list_data.is_some())
            {
                tracing::debug!(
                    target: WITHHELD_ACCESS_LIST_TARGET,
                    hash = %format_args!("{hash:#x}"),
                    "Withholding the delivered access list"
                );
            }
            None
        } else {
            delivered_access_list(block_fixture.block_access_list(), &block)
        };

        // Attempt to add the block as the head of the chain
        let chain_result = blockchain.add_block_pipeline(block.clone(), bal);

        match chain_result {
            Err(error) => {
                rejections.push(Rejection {
                    index,
                    hash: Some(hash),
                    error: error.to_string(),
                });
                if !expects_exception {
                    return Err(format!(
                        "Transaction execution unexpectedly failed on test: {test_key}, with error {error:?}",
                    ));
                }
                let expected_exception = block_fixture.expect_exception.clone().unwrap();
                if options.check_exception_names
                    && !exception_is_expected(expected_exception.clone(), &error)
                {
                    eprintln!(
                        "Warning: Returned exception {error:?} does not match expected {expected_exception:?}",
                    );
                }
                // Expected exception matched — block was rejected, but the test may
                // still expect subsequent blocks to be processed (e.g. fork-transition
                // tests where a block at the pre-fork timestamp fails and a block at
                // the post-fork timestamp succeeds, both built on the same parent).
                // Continue with the next block in the fixture.
                continue;
            }
            Ok(_) => {
                if expects_exception {
                    return Err(format!(
                        "Expected transaction execution to fail in test: {test_key} with error: {:?}",
                        block_fixture.expect_exception.clone()
                    ));
                }
                // Advance fork choice to the new head
                apply_fork_choice(store, hash, hash, hash, None)
                    .await
                    .unwrap();
            }
        }
    }

    // Final post-state verification
    check_poststate_against_db(test_key, test, store).await;
    Ok(())
}

/// Tracing target of the event the runner emits when it drops a delivered access
/// list, so a report can tell a dropped list from none.
pub const DROPPED_ACCESS_LIST_TARGET: &str = "ef_tests::dropped_access_list";

/// Tracing target of the event the runner emits when it withholds a block's list
/// (`RunOptions::withhold_access_lists`), so a report can tell a withheld list from
/// a block that has none.
pub const WITHHELD_ACCESS_LIST_TARGET: &str = "ef_tests::withheld_access_list";

/// The access list the fixture delivers with `block`, if the header commits to it.
///
/// Off the engine path a delivered list is a hint, not part of the block: one that
/// does not parse, or whose hash as delivered differs from `block_access_list_hash`,
/// is dropped and the block computes its own, as full sync does with a peer's list.
/// A list the header does commit to is attached even if it is malformed, and the
/// block is then judged with it.
pub fn delivered_access_list(
    delivered: Result<Option<BlockAccessList>, String>,
    block: &CoreBlock,
) -> Option<Arc<BlockAccessList>> {
    let bal = match delivered {
        Ok(None) => return None,
        Ok(Some(bal)) => Some(bal),
        Err(_) => None,
    }
    .filter(|bal| block.header.block_access_list_hash == Some(hash_as_delivered(bal)));
    if bal.is_none() {
        tracing::debug!(
            target: DROPPED_ACCESS_LIST_TARGET,
            hash = %format_args!("{:#x}", block.hash()),
            "Dropping the delivered access list"
        );
    }
    bal.map(Arc::new)
}

/// keccak of the list's RLP in the order given, duplicates and all.
/// `BlockAccessList`'s own encoding sorts every list, so a misordered list would
/// hash like the canonical one.
pub fn hash_as_delivered(bal: &BlockAccessList) -> H256 {
    struct Slot<'a>(&'a SlotChange);
    impl RLPEncode for Slot<'_> {
        fn encode(&self, buf: &mut dyn bytes::BufMut) {
            Encoder::new(buf)
                .encode_field(&self.0.slot)
                .encode_field(&self.0.slot_changes)
                .finish();
        }
    }
    struct Account<'a>(&'a AccountChanges);
    impl RLPEncode for Account<'_> {
        fn encode(&self, buf: &mut dyn bytes::BufMut) {
            let slots: Vec<Slot> = self.0.storage_changes.iter().map(Slot).collect();
            Encoder::new(buf)
                .encode_field(&self.0.address)
                .encode_field(&slots)
                .encode_field(&self.0.storage_reads)
                .encode_field(&self.0.balance_changes)
                .encode_field(&self.0.nonce_changes)
                .encode_field(&self.0.code_changes)
                .finish();
        }
    }
    let accounts: Vec<Account> = bal.accounts().iter().map(Account).collect();
    keccak(accounts.encode_to_vec())
}

/// Whether any block of the fixture carries its access list.
#[cfg(not(feature = "stateless"))]
fn delivers_access_lists(test: &TestUnit) -> bool {
    test.blocks.iter().any(|b| {
        b.block()
            .is_some_and(|block| block.block_access_list_data.is_some())
    })
}

/// Two-pass parallel execution check for Amsterdam tests.
///
/// Pass 1 (sequential): runs every block with `add_block_pipeline_bal` to collect the
/// BAL that each block produces.  Pass 2 (parallel): creates a fresh chain and re-runs every
/// block passing the corresponding BAL so the BAL-warmed parallel path is exercised.  The final
/// post-state of pass 2 must match the expected post-state. Both chains run with
/// `options`, like the one `run` imports with.
#[cfg(not(feature = "stateless"))]
async fn run_two_pass_parallel(
    test_key: &str,
    test: &TestUnit,
    options: &RunOptions,
) -> Result<(), String> {
    // ---- Pass 1: sequential, collect BALs ----
    let store1 = build_store_for_test(test).await;
    let blockchain1 = fixture_blockchain(store1.clone(), options);

    let mut bals: Vec<Arc<BlockAccessList>> = Vec::with_capacity(test.blocks.len());

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

        apply_fork_choice(&store1, hash, hash, hash, None)
            .await
            .map_err(|e| {
                format!("Two-pass pass-1 fork choice failed for test {test_key}: {e:?}")
            })?;

        // If execution produced no BAL (non-Amsterdam block in a transition test), skip pass 2.
        match produced_bal {
            Some(bal) => bals.push(Arc::new(bal)),
            None => return Ok(()),
        }
    }

    // ---- Pass 2: parallel (BAL-driven), verify post-state ----
    let store2 = build_store_for_test(test).await;
    let blockchain2 = fixture_blockchain(store2.clone(), options);

    for (block_fixture, bal) in test.blocks.iter().zip(bals.iter()) {
        let block: CoreBlock = block_fixture.block().unwrap().clone().into();
        let hash = block.hash();

        blockchain2
            .add_block_pipeline(block, Some(Arc::clone(bal)))
            .map_err(|e| format!("Two-pass pass-2 (parallel) failed for test {test_key}: {e:?}"))?;

        apply_fork_choice(&store2, hash, hash, hash, None)
            .await
            .map_err(|e| {
                format!("Two-pass pass-2 fork choice failed for test {test_key}: {e:?}")
            })?;
    }

    // Verify post-state matches expected
    check_poststate_against_db(test_key, test, &store2).await;
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
                // Legacy tx with out-of-range `v` (or out-of-range `r`/`s`): sender
                // recovery rejects the signature during execution.
                BlockChainExpectedException::InvalidSignature,
                ChainError::EvmError(EvmError::Transaction(_))
                    | ChainError::InvalidBlock(InvalidBlockError::InvalidTransaction(_))
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

    // A typed transaction whose `y_parity` is outside {0, 1} is rejected during
    // sender recovery, so `INVALID_SIGNATURE_VRS` is a legitimate reason for the
    // block to be rejected.
    let expects_invalid_signature = block_fixture
        .expect_exception
        .as_ref()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|case| matches!(case, BlockChainExpectedException::InvalidSignature));

    // A transaction nonce of 2^64 or greater does not fit ethrex's `u64` nonce field
    // (the nonce is a `u64` per the yellow paper / EIP-2681), so it is rejected at RLP
    // decoding with an `InvalidLength` error rather than at validation. EEST's
    // `NONCE_IS_MAX` fixtures (e.g. `tx_max_nonce`, nonce = 2^64) therefore fail decoding
    // here — a legitimate reason for the block to be rejected. (`create_transaction_high_nonce`
    // uses nonce = 2^64-1, which fits `u64`, decodes fine, and is caught later at validation.)
    let expects_nonce_too_high = block_fixture
        .expect_exception
        .as_ref()
        .unwrap_or(&Vec::new())
        .iter()
        .any(|case| matches!(case, BlockChainExpectedException::TxtException(msg) if msg == "Nonce is max"));

    match CoreBlock::decode(block_fixture.rlp.as_ref()) {
        Ok(_) => {
            assert!(!expects_rlp_exception);
            false
        }
        Err(_) => {
            assert!(expects_rlp_exception || expects_invalid_signature || expects_nonce_too_high);
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

/// Builds the blocks and a self-generated execution witness for a blockchain
/// test, using ethrex's own execution machinery instead of any witness the
/// fixture might carry. This is the ethrex-native counterpart to the
/// external eth-act `witness-generator-cli` path: it turns a plain EEST
/// `blockchain_test` (genesis/`pre` state + blocks, no embedded witness)
/// into a `(blocks, ExecutionWitness)` pair usable for stateless-execution
/// benchmarking.
///
/// Every block is first driven through `add_block_pipeline` (mirroring
/// `run()`), so that for multi-block tests the parent state of block
/// `i > 0` is already committed to the store before
/// `generate_witness_for_blocks` re-executes every block with a
/// `TrieLogger`. `generate_witness_for_blocks` only has the state at the
/// *first* block's parent available for free (from `build_store_for_test`);
/// its own doc comment notes it otherwise fails on the second block of a
/// batch.
///
/// A test whose blocks include an expected exception (invalid-block test) or
/// that decodes to zero blocks is rejected with an `Err` rather than
/// silently producing a partial/incorrect fixture, so batch callers (e.g.
/// `generate-stress`) can skip it and continue.
pub async fn blocks_and_witness_for_test(
    test: &TestUnit,
) -> Result<(Vec<CoreBlock>, ExecutionWitness), String> {
    let store = build_store_for_test(test).await;
    let blockchain = Blockchain::for_test_harness_with_pool(store.clone(), merkle_pool());

    let mut blocks: Vec<CoreBlock> = Vec::with_capacity(test.blocks.len());
    for block_fixture in test.blocks.iter() {
        if block_fixture.expect_exception.is_some() {
            return Err(
                "test has a block expecting an exception; not a witness-generation target"
                    .to_string(),
            );
        }
        let block_data = block_fixture
            .block()
            .ok_or_else(|| "block fixture has no decodable block (RLP-only test)".to_string())?;
        let block: CoreBlock = block_data.clone().into();
        let hash = block.hash();

        blockchain
            .add_block_pipeline(block.clone(), None)
            .map_err(|e| {
                format!(
                    "add_block_pipeline failed for block {}: {e:?}",
                    block.header.number
                )
            })?;
        apply_fork_choice(&store, hash, hash, hash, None)
            .await
            .map_err(|e| {
                format!(
                    "apply_fork_choice failed for block {}: {e:?}",
                    block.header.number
                )
            })?;

        blocks.push(block);
    }

    if blocks.is_empty() {
        return Err("no blocks".to_string());
    }

    let witness = blockchain
        .generate_witness_for_blocks(&blocks)
        .await
        .map_err(|e| format!("witness gen: {e}"))?;

    Ok((blocks, witness))
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
    let latest_block_number = db.get_latest_block_number().unwrap();
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
            if code_hash != *EMPTY_KECCAK_HASH {
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
    let last_block_number = db.get_latest_block_number().unwrap();
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

    // A generated witness has no spec wire bytes, so this cannot go through the
    // byte entrypoint; `ExecBackend` was also the only backend able to run an
    // in-memory witness, so the dispatch is gone with it.
    let execute_result = ethrex_guest_program::l1::validate_blocks_statelessly(
        &blocks,
        execution_witness,
        std::sync::Arc::new(ethrex_crypto::NativeCrypto),
    );

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
async fn run_stateless_from_fixture(test: &TestUnit, test_key: &str) -> Result<(), String> {
    let chain_config = test.network.chain_config();

    for block_fixture in test.blocks.iter() {
        // Skip blocks that expect exceptions — those are already validated by the normal path.
        if block_fixture.expect_exception.is_some() {
            continue;
        }

        let Some(block_data) = block_fixture.block() else {
            continue;
        };

        let block: CoreBlock = block_data.clone().into();
        let block_number = block.header.number;

        // Absent bytes means "expected to succeed"; malformed bytes are a hard error.
        let expected_valid = match block_data.stateless_output_bytes.as_deref() {
            None => true,
            Some(bytes) => parse_expected_valid_flag(bytes).map_err(|e| {
                format!("Malformed statelessOutputBytes for {test_key} block {block_number}: {e}")
            })?,
        };

        // Prefer the spec wire path — the same entrypoint the released guest ELF
        // runs — and compare the whole 43-byte result rather than peeking at the
        // validity byte. Only blocks that carry BOTH the input and the expected
        // output can go this way; the rest fall through to the witness route
        // below, which is why `parse_expected_valid_flag` is still needed.
        if let (Some(input_hex), Some(output_hex)) = (
            block_data.stateless_input_bytes.as_deref(),
            block_data.stateless_output_bytes.as_deref(),
        ) {
            run_stateless_from_input_bytes(test_key, block_number, input_hex, output_hex)?;
            continue;
        }

        let Some(witness_json) = block_data.execution_witness.as_ref() else {
            continue;
        };

        // Parse and conversion errors must always fail; only the execution outcome is
        // matched against `expected_valid` so the (false, Err(_)) arm below cannot
        // absorb regressions in deserialization or witness conversion.
        let rpc_witness: RpcExecutionWitness = serde_json::from_value(witness_json.clone())
            .map_err(|e| {
                format!("executionWitness parse failed for {test_key} block {block_number}: {e}")
            })?;
        let decoded_headers =
            ethrex_common::types::block_execution_witness::decode_witness_headers(
                &rpc_witness.headers,
            )
            .map_err(|e| {
                format!("witness header decode failed for {test_key} block {block_number}: {e}")
            })?;
        let execution_witness = rpc_witness
            .into_execution_witness(
                *chain_config,
                block_number,
                &decoded_headers,
                &ethrex_crypto::NativeCrypto,
            )
            .map_err(|e| {
                format!("witness conversion failed for {test_key} block {block_number}: {e}")
            })?;

        let exec_result = ethrex_guest_program::l1::validate_blocks_statelessly(
            std::slice::from_ref(&block),
            execution_witness,
            std::sync::Arc::new(ethrex_crypto::NativeCrypto),
        );

        match (expected_valid, exec_result) {
            (true, Ok(_)) | (false, Err(_)) => {}
            (true, Err(e)) => {
                return Err(format!(
                    "Stateless execution from fixture failed for {test_key} block {block_number}: {e}"
                ));
            }
            (false, Ok(_)) => {
                return Err(format!(
                    "Stateless execution from fixture succeeded for {test_key} block \
                     {block_number} but fixture expected it to fail (invalid executionWitness)"
                ));
            }
        }
    }

    Ok(())
}

/// Check the witness *generation* side: for every valid block carrying an
/// `executionWitness`, generate ethrex's own witness from the (already
/// executed) chain and diff its state/codes/headers sections against the
/// fixture's expected witness. `run_stateless_from_fixture` only proves the
/// fixture witness suffices for stateless execution; this proves ethrex
/// would have produced the canonical witness itself.
///
/// Blocks whose `statelessOutputBytes` mark the fixture witness as invalid
/// are skipped — their witness is deliberately corrupted and not a
/// generation target.
#[cfg(feature = "stateless")]
async fn check_witness_generation_against_fixture(
    blockchain: &Blockchain,
    test: &TestUnit,
    test_key: &str,
) -> Result<(), String> {
    use std::collections::BTreeSet;

    const MAX_REPORTED_ITEMS: usize = 8;

    // `errors` fail the test; `differences` are reported only. See the note at
    // the comparison below for why a witness need not equal the fixture's.
    let mut errors: Vec<String> = Vec::new();
    let mut differences: Vec<String> = Vec::new();
    for block_fixture in test.blocks.iter() {
        if block_fixture.expect_exception.is_some() {
            continue;
        }
        let Some(block_data) = block_fixture.block() else {
            continue;
        };
        let Some(witness_json) = block_data.execution_witness.as_ref() else {
            continue;
        };
        let expected_valid = match block_data.stateless_output_bytes.as_deref() {
            None => true,
            Some(bytes) => parse_expected_valid_flag(bytes)?,
        };
        if !expected_valid {
            continue;
        }

        let expected: RpcExecutionWitness = serde_json::from_value(witness_json.clone())
            .map_err(|e| format!("executionWitness parse failed for {test_key}: {e}"))?;

        let block: CoreBlock = block_data.clone().into();
        let block_number = block.header.number;
        let generated_witness = blockchain
            .generate_witness_for_blocks(std::slice::from_ref(&block))
            .await
            .map_err(|e| {
                format!("witness generation failed for {test_key} block {block_number}: {e}")
            })?;

        // Sufficiency: the generated witness must support stateless re-execution
        // of the block on its own, independent of how close it is to canonical.
        let exec_result = ethrex_guest_program::l1::validate_blocks_statelessly(
            std::slice::from_ref(&block),
            generated_witness.clone(),
            std::sync::Arc::new(ethrex_crypto::NativeCrypto),
        );
        if let Err(e) = exec_result {
            errors.push(format!(
                "{test_key} block {block_number}: generated witness INSUFFICIENT for \
                 stateless execution: {e}"
            ));
        }

        let generated = RpcExecutionWitness::try_from(generated_witness).map_err(|e| {
            format!("witness conversion failed for {test_key} block {block_number}: {e}")
        })?;

        for (section, got, exp) in [
            ("state", &generated.state, &expected.state),
            ("codes", &generated.codes, &expected.codes),
            ("headers", &generated.headers, &expected.headers),
        ] {
            // Order-insensitive comparison: canonical ordering is enforced at
            // serialization (`RpcExecutionWitness::try_from`), and some
            // fixtures (witness_validation `*_unsorted_but_complete`)
            // deliberately ship non-canonical order to test consumer leniency.
            let mut got_sorted: Vec<&[u8]> = got.iter().map(|b| b.as_ref()).collect();
            let mut exp_sorted: Vec<&[u8]> = exp.iter().map(|b| b.as_ref()).collect();
            got_sorted.sort();
            exp_sorted.sort();
            if got_sorted == exp_sorted {
                continue;
            }
            let got_set: BTreeSet<&[u8]> = got_sorted.iter().copied().collect();
            let exp_set: BTreeSet<&[u8]> = exp_sorted.iter().copied().collect();
            let missing: Vec<&&[u8]> = exp_set.difference(&got_set).collect();
            let extra: Vec<&&[u8]> = got_set.difference(&exp_set).collect();
            if missing.is_empty() && extra.is_empty() {
                differences.push(format!(
                    "{test_key} block {block_number} {section}: same item set but \
                     different multiplicity (generated {}, fixture {})",
                    got.len(),
                    exp.len()
                ));
                continue;
            }
            let fmt_items = |items: &[&&[u8]]| {
                let shown: Vec<String> = items
                    .iter()
                    .take(MAX_REPORTED_ITEMS)
                    .map(|b| describe_witness_item(section, b))
                    .collect();
                let suffix = if items.len() > MAX_REPORTED_ITEMS {
                    format!(" …and {} more", items.len() - MAX_REPORTED_ITEMS)
                } else {
                    String::new()
                };
                format!("{}{}", shown.join(", "), suffix)
            };
            let mut msg = format!(
                "{test_key} block {block_number} {section}: generated {} items, fixture {}.",
                got.len(),
                exp.len()
            );
            if !missing.is_empty() {
                msg.push_str(&format!(
                    " missing from generated: [{}]",
                    fmt_items(&missing)
                ));
            }
            if !extra.is_empty() {
                msg.push_str(&format!(" extra in generated: [{}]", fmt_items(&extra)));
            }
            // Reported, not failed. A witness only has to be *sufficient*, which
            // is asserted above by re-executing with it; it does not have to
            // equal the fixture's. EEST ships `validation_codes_extra_unused_bytecode`
            // (and the `state`/`headers` equivalents) precisely to pin that a
            // witness carrying extra unused items is valid, so neither direction
            // of this difference is a defect. Upstream bundles differ in policy
            // too: the zkevm conformance set excludes bytecode created inside the
            // block, while the zkevm-benchmark set includes it, so requiring
            // equality fails whichever one the client does not happen to mirror.
            differences.push(msg);
        }
    }

    if !differences.is_empty() {
        eprintln!(
            "note: {test_key} witness differs from the fixture's (both valid; the \
             generated one is proven sufficient above):\n{}",
            differences.join("\n")
        );
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "generated witness is not usable for stateless execution:\n{}",
            errors.join("\n")
        ))
    }
}

/// Render a witness item for mismatch reports: headers decode to their block
/// number, other sections show a hex prefix and length.
#[cfg(feature = "stateless")]
fn describe_witness_item(section: &str, bytes: &[u8]) -> String {
    if section == "headers"
        && let Ok(header) = CoreBlockHeader::decode(bytes)
    {
        return format!("header #{} ({} bytes)", header.number, bytes.len());
    }
    let prefix = hex::encode(&bytes[..bytes.len().min(8)]);
    format!("0x{prefix}… ({} bytes)", bytes.len())
}

/// Check every `engineNewPayloads` entry of a `blockchain_test_engine` fixture
/// file that carries stateless bytes: its `statelessInputBytes` must produce its
/// `statelessOutputBytes`.
///
/// Engine fixtures are a superset of the `blockchain_test` ones for stateless
/// purposes. They carry every stateless input the RLP-block fixtures do, plus the
/// payload mutations that only exist at the Engine API layer, such as a tampered
/// block access list or slot number, which cannot be expressed as an RLP block.
/// Only the bytes contract is checked here: the node-side execution of the same
/// payloads is the engine runner's job.
#[cfg(feature = "stateless")]
pub fn check_engine_stateless_bytes(path: &Path) -> datatest_stable::Result<()> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct EngineFixture {
        #[serde(default)]
        engine_new_payloads: Vec<EnginePayload>,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct EnginePayload {
        #[serde(default)]
        params: Vec<serde_json::Value>,
        stateless_input_bytes: Option<String>,
        stateless_output_bytes: Option<String>,
    }

    let raw = std::fs::read_to_string(path)?;
    let fixtures: HashMap<String, EngineFixture> = serde_json::from_str(&raw)?;

    let mut failures = Vec::new();
    for (test_key, fixture) in &fixtures {
        for payload in &fixture.engine_new_payloads {
            let (Some(input), Some(output)) = (
                payload.stateless_input_bytes.as_deref(),
                payload.stateless_output_bytes.as_deref(),
            ) else {
                continue;
            };
            // `params[0]` is the execution payload; its block number only labels
            // a failure, so an unreadable one falls back to 0 rather than failing.
            let block_number = payload
                .params
                .first()
                .and_then(|execution_payload| execution_payload.get("blockNumber"))
                .and_then(|number| number.as_str())
                .and_then(|hex| u64::from_str_radix(hex.trim_start_matches("0x"), 16).ok())
                .unwrap_or_default();
            if let Err(e) = run_stateless_from_input_bytes(test_key, block_number, input, output) {
                failures.push(e);
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n").into())
    }
}

/// Run a fixture's `statelessInputBytes` (2-byte BE schema-id followed by
/// SSZ-encoded `SszStatelessInput`) through the canonical-input path the
/// production guest binary uses.
#[cfg(feature = "stateless")]
fn run_stateless_from_input_bytes(
    test_key: &str,
    block_number: u64,
    input_hex: &str,
    expected_output_hex: &str,
) -> Result<(), String> {
    use ethrex_guest_program::l1::run_stateless_guest;

    let decode = |label: &str, hex_str: &str| {
        let trimmed = hex_str.strip_prefix("0x").unwrap_or(hex_str);
        hex::decode(trimmed).map_err(|e| {
            format!("{label} hex decode failed for {test_key} block {block_number}: {e}")
        })
    };

    let input = decode("statelessInputBytes", input_hex)?;
    let expected = decode("statelessOutputBytes", expected_output_hex)?;

    let actual = run_stateless_guest(&input, std::sync::Arc::new(ethrex_crypto::NativeCrypto));

    if actual != expected {
        return Err(format!(
            "statelessOutputBytes mismatch for {test_key} block {block_number}:\n  \
             expected 0x{}\n  actual   0x{}",
            hex::encode(&expected),
            hex::encode(&actual),
        ));
    }
    Ok(())
}

/// Decode the `valid` byte (index 32) from a zkevm-fixture `statelessOutputBytes` hex
/// string, encoded as `new_payload_request_root (32 B) ++ valid (1 B) ++ padding`.
#[cfg(feature = "stateless")]
fn parse_expected_valid_flag(hex: &str) -> Result<bool, String> {
    let trimmed = hex.strip_prefix("0x").unwrap_or(hex);
    let byte_hex = trimmed.get(64..66).ok_or_else(|| {
        format!(
            "expected at least 33 bytes (66 hex chars), got {} hex chars",
            trimmed.len()
        )
    })?;
    let byte = u8::from_str_radix(byte_hex, 16)
        .map_err(|e| format!("invalid hex at byte 32 ({byte_hex:?}): {e}"))?;
    match byte {
        0 => Ok(false),
        1 => Ok(true),
        n => Err(format!(
            "invalid validity byte 0x{n:02x} (expected 0x00 or 0x01)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_blockchain::payload::{BuildPayloadArgs, create_payload};
    use ethrex_common::{
        H160, H256,
        types::{DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Genesis},
    };

    async fn amsterdam_store() -> Store {
        let genesis_path = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../fixtures/genesis/l1-bal.json"
        ));
        let genesis = Genesis::try_from(genesis_path).expect("Failed to load genesis");
        let mut store =
            Store::new("store.db", EngineType::InMemory).expect("Failed to build store");
        store
            .add_initial_state(genesis)
            .await
            .expect("Failed to add genesis");
        store
    }

    /// An empty Amsterdam block on top of genesis and the access list it commits to.
    async fn amsterdam_block() -> (CoreBlock, BlockAccessList) {
        let store = amsterdam_store().await;
        let blockchain = Blockchain::for_test_harness_with_pool(store.clone(), merkle_pool());
        let genesis_header = store.get_block_header(0).unwrap().unwrap();
        let args = BuildPayloadArgs {
            parent: genesis_header.hash(),
            timestamp: genesis_header.timestamp + 12,
            fee_recipient: H160::zero(),
            random: H256::zero(),
            withdrawals: Some(Vec::new()),
            beacon_root: Some(H256::zero()),
            slot_number: Some(1),
            version: 1,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        };
        let block_template = create_payload(&args, &store, Bytes::new()).unwrap();
        let result = blockchain.build_payload(block_template).unwrap();
        let bal = result
            .block_access_list
            .expect("an Amsterdam block records an access list");
        (result.payload, bal)
    }

    async fn import(
        block: &CoreBlock,
        bal: Option<Arc<BlockAccessList>>,
        bal_parallel_exec: bool,
    ) -> Result<(), ChainError> {
        let mut blockchain =
            Blockchain::for_test_harness_with_pool(amsterdam_store().await, merkle_pool());
        blockchain.options.bal_parallel_exec_enabled = bal_parallel_exec;
        blockchain
            .add_block_pipeline(block.clone(), bal)
            .map(|_| ())
    }

    /// A delivered list the header does not commit to as delivered is dropped, and the
    /// block imports without it in both modes. One the header does commit to is
    /// attached, even out of order, and then judged with the block.
    #[tokio::test]
    async fn a_mismatched_delivered_list_is_dropped_and_the_block_imports() {
        let (block, bal) = amsterdam_block().await;
        // The block-start system calls touch several accounts, so the list can be
        // both shortened and reordered.
        assert!(bal.accounts().len() >= 2);
        let mut dropped = bal.accounts().to_vec();
        dropped.pop();
        let dropped = BlockAccessList::from_accounts(dropped);
        let mut reordered = bal.accounts().to_vec();
        reordered.reverse();
        let reordered = BlockAccessList::from_accounts(reordered);

        // For a list in canonical order the hash as delivered is the commitment.
        assert_eq!(
            Some(hash_as_delivered(&bal)),
            block.header.block_access_list_hash
        );
        assert!(delivered_access_list(Ok(Some(bal)), &block).is_some());
        // Attached, the shortened list fails the block on the parallel executor; that
        // is what dropping it avoids.
        assert!(
            import(&block, Some(Arc::new(dropped.clone())), true)
                .await
                .is_err()
        );

        for (name, delivered) in [
            ("an account dropped", Ok(Some(dropped))),
            ("accounts reordered", Ok(Some(reordered.clone()))),
            ("a list that does not parse", Err("unparseable".to_string())),
        ] {
            let used = delivered_access_list(delivered, &block);
            assert!(used.is_none(), "{name}: the list must be dropped");
            for parallel in [true, false] {
                let result = import(&block, used.clone(), parallel).await;
                assert!(
                    result.is_ok(),
                    "{name}, parallel={parallel}: the block must import, got {result:?}"
                );
            }
        }

        // A header committing to the reordered bytes gets the reordered list, which
        // makes the block invalid.
        let mut committed = block.clone();
        committed.header.block_access_list_hash = Some(hash_as_delivered(&reordered));
        let used = delivered_access_list(Ok(Some(reordered)), &committed);
        assert!(
            used.is_some(),
            "a list the header commits to must be attached"
        );
        for parallel in [true, false] {
            let result = import(&committed, used.clone(), parallel).await;
            assert!(
                result.is_err(),
                "parallel={parallel}: a block committing to a misordered list must be rejected"
            );
        }
    }
}
