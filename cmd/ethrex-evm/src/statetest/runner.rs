//! Execution loop for the `statetest` subcommand.
//!
//! Reads JSON state-test files, runs each (fork, subtest) pair through LEVM,
//! and streams EIP-3155 trace lines + the goevmlab terminator to stderr.

use std::{
    collections::BTreeMap,
    io::{self, BufRead},
    path::{Path, PathBuf},
};

use ethrex_common::{
    Address, H256, U256,
    types::{
        Account, AccountInfo, Code, EIP1559Transaction, Fork, Genesis, GenesisAccount, Transaction,
        TxKind, tx_fields::AccessList,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    EVMConfig, Environment,
    tracing::{LevmOpcodeTracer, OpcodeTracerConfig},
    utils::get_base_fee_per_blob_gas,
    vm::{VM, VMType},
};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::backends;
use regex::Regex;
use rustc_hash::FxHashMap;
use walkdir::WalkDir;

use crate::statetest::{
    StatetestArgs,
    error_map::vm_error_to_geth_string,
    state_root::{build_generalized_db, compute_post_state_root},
    types::{StateTestAccount, StateTestFile},
};

/// Entry point for the `statetest` subcommand.
pub fn run(args: StatetestArgs) -> eyre::Result<()> {
    // Validate trace format early — geth exits 1 on unsupported values.
    if args.trace && args.trace_format != "json" {
        eprintln!(
            "unsupported trace format: {}; only \"json\" is supported",
            args.trace_format
        );
        std::process::exit(1);
    }

    let files = collect_files(&args.paths)?;

    let run_re = Regex::new(args.run.as_deref().unwrap_or(""))
        .map_err(|e| eyre::eyre!("invalid --run regex: {e}"))?;

    for file in &files {
        run_file(file, &args, &run_re).map_err(|e| eyre::eyre!("file {}: {e}", file.display()))?;
    }

    Ok(())
}

/// Collects `.json` files from paths. Directories are walked recursively.
/// When `paths` is empty, reads newline-separated paths from stdin.
fn collect_files(paths: &[PathBuf]) -> eyre::Result<Vec<PathBuf>> {
    if paths.is_empty() {
        // Batch mode: read paths from stdin, one per line.
        let stdin = io::stdin();
        let mut files = Vec::new();
        for line in stdin.lock().lines() {
            let line = line.map_err(|e| eyre::eyre!("reading stdin: {e}"))?;
            let line = line.trim().to_owned();
            if line.is_empty() {
                break;
            }
            files.extend(collect_from_path(Path::new(&line)));
        }
        Ok(files)
    } else {
        let mut files = Vec::new();
        for p in paths {
            files.extend(collect_from_path(p));
        }
        Ok(files)
    }
}

/// Returns all `.json` files under `path` (or just `path` if it is a file).
fn collect_from_path(path: &Path) -> Vec<PathBuf> {
    if path.is_dir() {
        WalkDir::new(path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().is_file()
                    && e.path().extension().and_then(|s| s.to_str()) == Some("json")
            })
            .map(|e| e.path().to_owned())
            .collect()
    } else {
        vec![path.to_owned()]
    }
}

fn run_file(path: &Path, args: &StatetestArgs, run_re: &Regex) -> eyre::Result<()> {
    let src = std::fs::read_to_string(path)?;
    let file: StateTestFile =
        serde_json::from_str(&src).map_err(|e| eyre::eyre!("parse error: {e}"))?;

    for (test_name, test) in &file {
        if !run_re.is_match(test_name) {
            continue;
        }
        for (fork_name, post_vectors) in &test.post {
            if let Some(ref wanted_fork) = args.statetest_fork
                && fork_name != wanted_fork
            {
                continue;
            }
            let fork = parse_fork(fork_name)?;

            for (idx, vector) in post_vectors.iter().enumerate() {
                if let Some(wanted_idx) = args.statetest_index
                    && idx != wanted_idx
                {
                    continue;
                }

                run_subtest(
                    args,
                    &test.pre,
                    &test.env,
                    &test.transaction,
                    fork,
                    vector,
                    idx,
                )?;
            }
        }
    }
    Ok(())
}

fn run_subtest(
    args: &StatetestArgs,
    pre: &BTreeMap<Address, StateTestAccount>,
    env: &crate::statetest::types::TestEnv,
    tx_template: &crate::statetest::types::TestTransaction,
    fork: Fork,
    vector: &crate::statetest::types::PostStateVector,
    _idx: usize,
) -> eyre::Result<()> {
    // Build pre-state as FxHashMap<Address, Account>.
    let pre_state = build_pre_state(pre);

    // Build genesis and store for this subtest.
    let genesis = build_genesis_from_pre(&pre_state, env);
    let rt = tokio::runtime::Runtime::new()?;
    let (store, _block_hash) = rt.block_on(async {
        let mut store =
            Store::new("./temp", EngineType::InMemory).map_err(|e| eyre::eyre!("{e}"))?;
        store
            .add_initial_state(genesis.clone())
            .await
            .map_err(|e| eyre::eyre!("{e}"))?;
        let block_hash = genesis.get_block().hash();
        Ok::<_, eyre::Error>((store, block_hash))
    })?;

    let mut db = build_generalized_db(store, &genesis)?;

    // Build the transaction from the template + subtest indexes.
    let data = tx_template
        .data
        .get(vector.indexes.data)
        .cloned()
        .unwrap_or_default();
    let gas_limit = tx_template
        .gas_limit
        .get(vector.indexes.gas)
        .copied()
        .unwrap_or_default();
    let value = tx_template
        .value
        .get(vector.indexes.value)
        .copied()
        .unwrap_or(U256::zero());

    let to = match tx_template.to {
        Some(addr) => TxKind::Call(addr),
        None => TxKind::Create,
    };

    let access_list: AccessList = parse_access_list(&tx_template.access_lists, vector.indexes.data);

    // Determine gas price / fee fields.
    let (gas_price_u256, max_fee_per_gas_u256, max_priority_fee_per_gas_u256) =
        compute_fee_fields(tx_template, env)?;

    // Recover sender from secret_key.
    let sender = recover_sender(tx_template)?;

    let blob_schedule = EVMConfig::canonical_values(fork);
    let config = EVMConfig::new(fork, blob_schedule);

    let base_blob_fee_per_gas =
        get_base_fee_per_blob_gas(None, &config).map_err(|e| eyre::eyre!("base blob fee: {e}"))?;

    // Mirror tooling/ef_tests/state/runner/levm_runner.rs::prepare_vm_for_tx:
    // always wrap in EIP1559Transaction with default fee fields. LEVM's
    // execution layer reads the effective price from `Environment` (gas_price
    // / tx_max_fee_per_gas) rather than the envelope, so legacy vectors work
    // through this branch without any fee-math drift.
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to,
        value,
        data,
        access_list,
        gas_limit,
        ..Default::default()
    });

    let levm_env = Environment {
        origin: sender,
        gas_limit,
        config,
        block_number: env.current_number,
        coinbase: env.current_coinbase,
        timestamp: env.current_timestamp,
        prev_randao: env.current_random,
        difficulty: env.current_difficulty,
        slot_number: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: env.current_base_fee.unwrap_or(U256::zero()),
        base_blob_fee_per_gas,
        gas_price: gas_price_u256,
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: max_priority_fee_per_gas_u256,
        tx_max_fee_per_gas: max_fee_per_gas_u256,
        tx_max_fee_per_blob_gas: None,
        tx_nonce: tx_template.nonce,
        block_gas_limit: env.current_gas_limit,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: false,
    };

    // Build tracer.
    let mut tracer = if args.trace {
        let cfg = OpcodeTracerConfig {
            disable_stack: args.trace_nostack,
            enable_memory: args.trace_memory,
            disable_storage: args.trace_nostorage,
            enable_return_data: !args.trace_noreturndata,
            limit: 0,
        };
        LevmOpcodeTracer::streaming(cfg, Box::new(std::io::stderr()))
    } else {
        LevmOpcodeTracer::disabled()
    };

    // Execute.
    let call_tracer = ethrex_levm::tracing::LevmCallTracer::disabled();
    let exec_result = VM::new(
        levm_env,
        &mut db,
        &tx,
        call_tracer,
        VMType::L1,
        &NativeCrypto,
    )
    .map_err(|e| eyre::eyre!("VM init: {e}"))
    .map(|mut vm| {
        vm.opcode_tracer = std::mem::replace(&mut tracer, LevmOpcodeTracer::disabled());
        let result = vm.execute();
        tracer = std::mem::replace(&mut vm.opcode_tracer, LevmOpcodeTracer::disabled());
        result
    });

    let (output, gas_used, error_str) = match &exec_result {
        Ok(Ok(report)) => {
            let err = match &report.result {
                ethrex_levm::errors::TxResult::Revert(e) => Some(vm_error_to_geth_string(e)),
                ethrex_levm::errors::TxResult::Success => None,
            };
            (report.output.to_vec(), report.gas_spent, err)
        }
        Ok(Err(vm_err)) => {
            let err_str = vm_error_to_geth_string(vm_err);
            (vec![], 0, Some(err_str))
        }
        Err(e) => {
            return Err(eyre::eyre!("VM setup: {e}"));
        }
    };

    tracer.flush_summary(&output, gas_used, error_str.as_deref())?;

    // Get state transitions and compute post-state root.
    let updates = backends::levm::LEVM::get_state_transitions(&mut db)
        .map_err(|e| eyre::eyre!("get_state_transitions: {e}"))?;

    let post_root = compute_post_state_root(&pre_state, &updates)?;

    // Always emit the state root terminator to stderr — goevmlab reads this line
    // to detect test completion regardless of whether per-step tracing is enabled.
    // Mirror: go-ethereum/cmd/evm/staterunner.go fmt.Fprintf(os.Stderr, ...).
    tracer.flush_state_root(post_root)?;
    if !args.trace {
        // When the tracer has no sink, write the terminator directly.
        use ethrex_common::tracing::write_streaming_state_root;
        use std::io::Write as _;
        let mut stderr = std::io::stderr();
        write_streaming_state_root(&mut stderr, post_root)
            .map_err(|e| eyre::eyre!("state root write: {e}"))?;
        stderr
            .flush()
            .map_err(|e| eyre::eyre!("stderr flush: {e}"))?;
    }

    if let Some(stream_err) = tracer.take_stream_error() {
        return Err(eyre::eyre!("stream write error: {stream_err}"));
    }

    Ok(())
}

/// Parses a fork name string into a [`Fork`] variant.
fn parse_fork(name: &str) -> eyre::Result<Fork> {
    // The statetest JSON uses geth's fork naming (e.g. "Prague", "Cancun").
    match name {
        "Frontier" => Ok(Fork::Frontier),
        "FrontierToHomesteadAt5" | "Homestead" => Ok(Fork::Homestead),
        "HomesteadToDaoAt5" | "DaoFork" => Ok(Fork::DaoFork),
        "EIP150" | "Tangerine" => Ok(Fork::Tangerine),
        "EIP158" | "SpuriousDragon" => Ok(Fork::SpuriousDragon),
        "Byzantium" => Ok(Fork::Byzantium),
        "Constantinople" => Ok(Fork::Constantinople),
        "ConstantinopleFix" | "Petersburg" => Ok(Fork::Petersburg),
        "Istanbul" => Ok(Fork::Istanbul),
        "MuirGlacier" => Ok(Fork::MuirGlacier),
        "Berlin" => Ok(Fork::Berlin),
        "London" => Ok(Fork::London),
        "ArrowGlacier" => Ok(Fork::ArrowGlacier),
        "GrayGlacier" => Ok(Fork::GrayGlacier),
        "Merge" | "Paris" | "MergeEOF" => Ok(Fork::Paris),
        "Shanghai" => Ok(Fork::Shanghai),
        "Cancun" => Ok(Fork::Cancun),
        "Prague" => Ok(Fork::Prague),
        "Osaka" => Ok(Fork::Osaka),
        other => Err(eyre::eyre!("unknown fork: {other}")),
    }
}

/// Converts a `BTreeMap<Address, StateTestAccount>` into `FxHashMap<Address, Account>`.
fn build_pre_state(pre: &BTreeMap<Address, StateTestAccount>) -> FxHashMap<Address, Account> {
    let crypto = NativeCrypto;
    pre.iter()
        .map(|(addr, sta)| {
            let code = Code::from_bytecode(sta.code.clone(), &crypto);
            let code_hash = code.hash;
            let storage: FxHashMap<H256, U256> = sta
                .storage
                .iter()
                .map(|(k, v)| (H256::from_slice(&k.to_big_endian()), *v))
                .collect();
            let account = Account {
                info: AccountInfo {
                    balance: sta.balance,
                    nonce: sta.nonce,
                    code_hash,
                },
                code,
                storage,
            };
            (*addr, account)
        })
        .collect()
}

/// Builds a [`Genesis`] from a pre-state map, using the block env for gas_limit.
fn build_genesis_from_pre(
    pre_state: &FxHashMap<Address, Account>,
    env: &crate::statetest::types::TestEnv,
) -> Genesis {
    let alloc: BTreeMap<Address, GenesisAccount> = pre_state
        .iter()
        .map(|(addr, account)| {
            let storage: BTreeMap<U256, U256> = account
                .storage
                .iter()
                .map(|(k, v)| (U256::from_big_endian(k.as_bytes()), *v))
                .collect();
            let ga = GenesisAccount {
                code: account.code.bytecode.clone(),
                storage,
                balance: account.info.balance,
                nonce: account.info.nonce,
            };
            (*addr, ga)
        })
        .collect();

    // Use the default ChainConfig (all forks inactive in the genesis header);
    // LEVM's `EVMConfig` is what drives fork-specific behavior at exec time.
    // Mirroring tooling/ef_tests/state's `Genesis::from(&EFTest)`.
    Genesis {
        alloc,
        gas_limit: env.current_gas_limit,
        coinbase: env.current_coinbase,
        difficulty: env.current_difficulty,
        mix_hash: env.current_random.unwrap_or_default(),
        timestamp: env.current_timestamp,
        base_fee_per_gas: env
            .current_base_fee
            .map(|v| v.try_into().unwrap_or(u64::MAX)),
        ..Default::default()
    }
}

/// Computes (gas_price, max_fee_per_gas, max_priority_fee_per_gas) from the
/// transaction template and block environment.
fn compute_fee_fields(
    tx: &crate::statetest::types::TestTransaction,
    env: &crate::statetest::types::TestEnv,
) -> eyre::Result<(U256, Option<U256>, Option<U256>)> {
    match tx.gas_price {
        Some(price) => {
            // Legacy / EIP-2930: effective gas price == gas_price.
            Ok((price, tx.max_fee_per_gas, tx.max_priority_fee_per_gas))
        }
        None => {
            // EIP-1559: effective = min(max_fee, base_fee + priority).
            let base_fee = env
                .current_base_fee
                .ok_or_else(|| eyre::eyre!("EIP-1559 tx but no currentBaseFee in env"))?;
            let max_priority = tx
                .max_priority_fee_per_gas
                .ok_or_else(|| eyre::eyre!("EIP-1559 tx missing maxPriorityFeePerGas"))?;
            let max_fee = tx
                .max_fee_per_gas
                .ok_or_else(|| eyre::eyre!("EIP-1559 tx missing maxFeePerGas"))?;
            let effective = std::cmp::min(max_fee, base_fee + max_priority);
            Ok((effective, Some(max_fee), Some(max_priority)))
        }
    }
}

/// Parses the access_lists JSON array at the given data index into an [`AccessList`].
///
/// The `access_lists` field in the statetest JSON is an array-of-arrays; each
/// inner element is an access list for one `data` index.  When the field is
/// absent or has no entry for this index we return an empty list.
fn parse_access_list(raw: &[serde_json::Value], data_idx: usize) -> AccessList {
    let entry = match raw.get(data_idx) {
        Some(v) => v,
        None => return vec![],
    };

    // Each entry is an array of { "address": "0x...", "storageKeys": ["0x...",...] }
    let items = match entry.as_array() {
        Some(a) => a,
        None => return vec![],
    };

    items
        .iter()
        .filter_map(|item| {
            let addr_str = item["address"].as_str()?;
            let addr: Address = addr_str.parse().ok()?;
            let keys: Vec<H256> = item["storageKeys"]
                .as_array()
                .map(|ks| {
                    ks.iter()
                        .filter_map(|k| k.as_str()?.parse::<H256>().ok())
                        .collect()
                })
                .unwrap_or_default();
            Some((addr, keys))
        })
        .collect()
}

/// Derives the sender address from the test's `secretKey`.
///
/// EF statetests include the private key so the sender can be derived without
/// a signature: compute the uncompressed public key, keccak256 it, take
/// the last 20 bytes as the Ethereum address.
fn recover_sender(tx: &crate::statetest::types::TestTransaction) -> eyre::Result<Address> {
    use ethrex_crypto::keccak::keccak_hash;
    use secp256k1::{PublicKey, SECP256K1, SecretKey};

    let sk = SecretKey::from_slice(tx.secret_key.as_bytes())
        .map_err(|e| eyre::eyre!("invalid secret key: {e}"))?;
    let pubkey = PublicKey::from_secret_key(SECP256K1, &sk);
    // Uncompressed public key: 65 bytes, first byte is 0x04 (prefix), skip it.
    let uncompressed = pubkey.serialize_uncompressed();
    let hash = keccak_hash(&uncompressed[1..]);
    // Address is the last 20 bytes of the 32-byte keccak hash.
    Ok(Address::from_slice(&hash[12..]))
}
