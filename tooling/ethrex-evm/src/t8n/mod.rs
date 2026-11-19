//! Geth-compatible t8n (state transition) tool backed by LEVM.
//!
//! Reads a pre-state alloc, block environment, and transaction list; builds
//! the block through the same payload-building pipeline the node uses
//! (`Blockchain::build_payload_t8n`) on an in-memory store; and writes the
//! post-state alloc and execution result in geth's `evm t8n` output formats.
//!
//! From Amsterdam on, when the environment has the ancestor headers
//! (`blockHeaders`), the result also has the execution witness of the block
//! and whether the block validates statelessly against it.

pub mod fork;

mod error;
mod output;
mod tx;
mod types;
mod witness;

use std::path::PathBuf;

use bytes::Bytes;
use clap::Args;
use ethrex_blockchain::payload::{PayloadBuildResult, T8nBuildOptions};
use ethrex_blockchain::{Blockchain, BlockchainOptions, BlockchainType};
use ethrex_common::constants::DEFAULT_OMMERS_HASH;
use ethrex_common::types::{
    BlobSchedule, Block, BlockBody, BlockHeader, ELASTICITY_MULTIPLIER, ForkBlobSchedule, Genesis,
    Transaction, calc_excess_blob_gas, calculate_base_fee_per_gas, compute_receipts_root,
    compute_transactions_root, compute_withdrawals_root,
};
use ethrex_common::validation::validate_block_access_list_size;
use ethrex_common::{Bloom, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{EngineType, Store};

use error::T8nError;
use output::{Rejection, WitnessResult};
use types::Inputs;
use witness::WitnessOutput;

#[derive(Args)]
pub struct T8nArgs {
    /// Pre-state alloc JSON: a file path, or `stdin` to read it from the
    /// stdin input bundle.
    #[arg(long = "input.alloc", default_value = "alloc.json")]
    pub input_alloc: String,
    /// Block environment JSON: a file path or `stdin`.
    #[arg(long = "input.env", default_value = "env.json")]
    pub input_env: String,
    /// Transactions JSON array: a file path or `stdin`.
    #[arg(long = "input.txs", default_value = "txs.json")]
    pub input_txs: String,
    /// Blob parameters JSON file for the block's fork; when omitted, the
    /// stdin bundle's `blobParams` key or the fork's canonical values apply.
    #[arg(long = "input.blobParams")]
    pub input_blob_params: Option<String>,
    /// Base directory for relative output paths.
    #[arg(long = "output.basedir", default_value = "")]
    pub output_basedir: PathBuf,
    /// Where to write the execution result JSON.
    #[arg(long = "output.result", default_value = "result.json")]
    pub output_result: String,
    /// Where to write the post-state alloc JSON.
    #[arg(long = "output.alloc", default_value = "alloc.json")]
    pub output_alloc: String,
    /// Where to write the RLP of the included transactions.
    #[arg(long = "output.body", default_value = "txs.rlp")]
    pub output_body: String,
    /// Fork to execute under; see the supported list below.
    #[arg(long = "state.fork")]
    pub fork: String,
    /// Chain id.
    #[arg(long = "state.chainid", default_value_t = 1)]
    pub chain_id: u64,
    /// Mining reward. Only `0` and `-1` (no reward) are accepted: pre-merge
    /// forks are not supported.
    #[arg(long = "state.reward", default_value_t = 0, allow_hyphen_values = true)]
    pub reward: i64,
    /// State-test semantics: apply only the transactions, with no system
    /// operations (no beacon-root/history calls, requests, or withdrawals).
    #[arg(long = "state-test", default_value_t = false)]
    pub state_test: bool,
    /// Write how many times each opcode ran in the block to this file, as a
    /// JSON object from opcode name to count. Relative to `--output.basedir`.
    #[arg(long = "opcode.count")]
    pub opcode_count: Option<String>,
}

impl T8nArgs {
    /// Where `--opcode.count` writes the counts, if it has to. Geth takes an
    /// empty file name as the flag not being set.
    fn opcode_count_path(&self) -> Option<&str> {
        self.opcode_count.as_deref().filter(|path| !path.is_empty())
    }
}

/// What a t8n run computes from its inputs, before it is written out.
pub struct Execution {
    pub result: PayloadBuildResult,
    /// The rejected transactions, by their index in the input list.
    pub rejections: Vec<Rejection>,
    pub block_exception: Option<String>,
    /// Set when the run generates the execution witness.
    pub witness: Option<WitnessResult>,
    /// Set with `--opcode.count`: the opcodes that ran, with their counts.
    pub opcode_counts: Option<Vec<(u8, u64)>>,
}

pub fn run(args: T8nArgs) -> Result<(), T8nError> {
    if args.reward > 0 {
        return Err(T8nError::Unsupported(
            "positive block rewards (pre-merge forks) are not supported".to_string(),
        ));
    }
    let inputs = types::read_inputs(&args)?;
    let execution = execute(&args, &inputs)?;
    output::write_outputs(&args, &inputs.alloc, &execution)
}

fn execute(args: &T8nArgs, inputs: &Inputs) -> Result<Execution, T8nError> {
    let fork = fork::parse_fork(&args.fork)?;
    let mut config = fork::chain_config_for(fork, args.chain_id);
    // Filler-provided blob parameters override the canonical schedule for
    // whichever fork resolves at the block's timestamp.
    if let Some(params) = &inputs.blob_params {
        let to_u64 = |value: U256, field: &str| -> Result<u64, T8nError> {
            u64::try_from(value)
                .map_err(|_| T8nError::Parse("blobParams".to_string(), field.to_string()))
        };
        let schedule = ForkBlobSchedule {
            target: to_u64(params.target, "target")? as u32,
            max: to_u64(params.max, "max")? as u32,
            base_fee_update_fraction: to_u64(
                params.base_fee_update_fraction,
                "baseFeeUpdateFraction",
            )?,
        };
        config.blob_schedule = BlobSchedule {
            cancun: schedule,
            prague: schedule,
            osaka: schedule,
            bpo1: schedule,
            bpo2: schedule,
            bpo3: Some(schedule),
            bpo4: Some(schedule),
            bpo5: Some(schedule),
            amsterdam: Some(schedule),
        };
    }
    let env = &inputs.env;
    let timestamp = env.current_timestamp;
    let block_hash_cache = env.parsed_block_hashes()?;

    // Parse and convert transactions individually: a malformed transaction
    // becomes a `rejected` entry, not a tool failure.
    let mut rejections: Vec<Rejection> = Vec::new();
    let mut transactions: Vec<Transaction> = Vec::new();
    let mut input_indices: Vec<usize> = Vec::new();
    for (index, value) in inputs.txs.iter().enumerate() {
        let converted = serde_json::from_value::<tx::TxJson>(value.clone())
            .map_err(|e| e.to_string())
            .and_then(|tx_json| tx::to_ethrex_transaction(&tx_json));
        match converted {
            Ok(transaction) => {
                transactions.push(transaction);
                input_indices.push(index);
            }
            Err(error) => rejections.push(Rejection { index, error }),
        }
    }

    // The pre-state alloc becomes the genesis of an in-memory store; the
    // genesis block acts as the parent the payload executes on top of.
    let genesis = Genesis {
        config,
        alloc: inputs.alloc.clone(),
        coinbase: Default::default(),
        difficulty: U256::zero(),
        extra_data: Bytes::new(),
        gas_limit: env.parent_gas_limit.unwrap_or(env.current_gas_limit),
        nonce: 0,
        mix_hash: H256::zero(),
        timestamp: env
            .parent_timestamp
            .unwrap_or_else(|| timestamp.saturating_sub(12)),
        base_fee_per_gas: env.parent_base_fee,
        blob_gas_used: env.parent_blob_gas_used,
        excess_blob_gas: env.parent_excess_blob_gas,
        requests_hash: None,
        block_access_list_hash: None,
        slot_number: env.parent_slot_number,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| T8nError::Store(e.to_string()))?;
    let mut store = Store::new("ethrex-evm-t8n", EngineType::InMemory)
        .map_err(|e| T8nError::Store(e.to_string()))?;
    runtime
        .block_on(store.add_initial_state(genesis.clone()))
        .map_err(|e| T8nError::Store(e.to_string()))?;
    let parent_header = genesis.get_block().header;
    // The witness has the headers of the block's ancestors, and the t8n can
    // only get the real ones from the environment. State tests skip the
    // system operations of a block, so what they build can't be validated
    // statelessly and gets no witness.
    let with_witness = !args.state_test
        && config.is_amsterdam_activated(timestamp)
        && !env.block_headers.is_empty();
    let genesis_hash = parent_header.hash();
    let parent_hash = if with_witness {
        // The real ancestors go in the store, the parent among them, and
        // the block is built on top of that parent.
        let ancestors = env.parsed_block_headers(&block_hash_cache)?;
        runtime.block_on(witness::store_ancestor_headers(
            &store,
            ancestors,
            env.current_number,
            parent_header.state_root,
        ))?
    } else {
        // Prefer the environment's real parent hash (`blockHashes[number-1]`)
        // over the fabricated genesis hash: the EIP-2935 system call writes
        // the payload's parent hash into the history contract, so it must
        // match the test's chain. The genesis-state header is aliased under
        // that hash so parent lookups still resolve.
        let parent_hash = block_hash_cache
            .get(&env.current_number.wrapping_sub(1))
            .copied()
            .unwrap_or(genesis_hash);
        if parent_hash != genesis_hash {
            runtime
                .block_on(store.add_block_header(parent_hash, parent_header.clone()))
                .map_err(|e| T8nError::Store(e.to_string()))?;
        }
        parent_hash
    };

    let base_fee_per_gas = env.current_base_fee.or_else(|| {
        calculate_base_fee_per_gas(
            env.current_gas_limit,
            env.parent_gas_limit.unwrap_or(env.current_gas_limit),
            env.parent_gas_used.unwrap_or_default(),
            env.parent_base_fee.unwrap_or_default(),
            ELASTICITY_MULTIPLIER,
        )
    });
    let excess_blob_gas = env.current_excess_blob_gas.or_else(|| {
        config
            .get_fork_blob_schedule(timestamp)
            .map(|schedule| calc_excess_blob_gas(&parent_header, schedule, config.fork(timestamp)))
    });
    let withdrawals = config
        .is_shanghai_activated(timestamp)
        .then(|| env.withdrawals.clone().unwrap_or_default());

    // The header is pinned to the environment's values rather than derived
    // from the parent; execution-derived fields (roots, gas used, bloom,
    // BAL hash) are placeholders that `finalize_payload` overwrites.
    let header = BlockHeader {
        parent_hash,
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase: env.current_coinbase,
        state_root: parent_header.state_root,
        transactions_root: compute_transactions_root(&[], &NativeCrypto),
        receipts_root: compute_receipts_root(&[], &NativeCrypto),
        logs_bloom: Bloom::default(),
        difficulty: U256::zero(),
        number: env.current_number,
        gas_limit: env.current_gas_limit,
        gas_used: 0,
        timestamp,
        extra_data: Bytes::new(),
        prev_randao: H256::from(env.current_random.unwrap_or_default().to_big_endian()),
        nonce: 0,
        base_fee_per_gas,
        withdrawals_root: withdrawals
            .as_ref()
            .map(|withdrawals| compute_withdrawals_root(withdrawals, &NativeCrypto)),
        blob_gas_used: config.is_cancun_activated(timestamp).then_some(0),
        excess_blob_gas,
        parent_beacon_block_root: env.parent_beacon_block_root,
        slot_number: config
            .is_amsterdam_activated(timestamp)
            .then(|| env.slot_number.unwrap_or(0)),
        ..Default::default()
    };
    let body = BlockBody {
        transactions: Vec::new(),
        ommers: Vec::new(),
        withdrawals,
    };

    let blockchain = Blockchain::new(
        store.clone(),
        BlockchainOptions {
            r#type: BlockchainType::L1,
            ..Default::default()
        },
    );
    let build = blockchain
        .build_payload_t8n(
            Block::new(header, body),
            transactions,
            T8nBuildOptions {
                block_hash_cache,
                state_test: args.state_test,
                witness: with_witness,
                opcode_count: args.opcode_count_path().is_some(),
            },
        )
        .map_err(|e| T8nError::Build(e.to_string()))?;
    let result = build.result;

    // The t8n doesn't have the final block of the caller, so it validates
    // the one it built.
    let witness = build
        .witness
        .map(|witness| {
            let witness = WitnessOutput::new(witness)?;
            let validation = witness.validate(&result.payload, config);
            if let Err(error) = &validation {
                eprintln!("stateless validation of the built block failed: {error}");
            }
            Ok::<_, T8nError>(WitnessResult {
                witness,
                successful_validation: validation.is_ok(),
            })
        })
        .transpose()?;

    // Map build-time rejection indices (into the converted list) back to
    // input positions and merge with the conversion-time rejections.
    for rejection in build.rejected {
        rejections.push(Rejection {
            index: input_indices[rejection.index],
            error: rejection.error,
        });
    }
    rejections.sort_by_key(|rejection| rejection.index);

    // Block-level validity that only shows post-build: reported as
    // `blockException` so fillers can match expected-invalid blocks.
    let block_exception = build.block_exception.or_else(|| {
        result.block_access_list.as_ref().and_then(|bal| {
            validate_block_access_list_size(&result.payload.header, &config, bal)
                .err()
                .map(|error| error.to_string())
        })
    });

    Ok(Execution {
        result,
        rejections,
        block_exception,
        witness,
        opcode_counts: build.opcode_counts,
    })
}

#[cfg(test)]
mod tests;
