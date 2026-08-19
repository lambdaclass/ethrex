//! Geth-compatible t8n (state transition) tool backed by LEVM.
//!
//! Reads a pre-state alloc, block environment, and transaction list; builds
//! the block through the same payload-building pipeline the node uses
//! (`Blockchain::build_payload_t8n`) on an in-memory store; and writes the
//! post-state alloc and execution result in geth's `evm t8n` output formats.

pub mod fork;

mod error;
mod output;
mod tx;
mod types;

use std::path::PathBuf;

use bytes::Bytes;
use clap::Args;
use ethrex_blockchain::{Blockchain, BlockchainOptions, BlockchainType};
use ethrex_common::constants::DEFAULT_OMMERS_HASH;
use ethrex_common::types::{
    Block, BlockBody, BlockHeader, ELASTICITY_MULTIPLIER, Genesis, Transaction,
    calc_excess_blob_gas, calculate_base_fee_per_gas, compute_receipts_root,
    compute_transactions_root, compute_withdrawals_root,
};
use ethrex_common::validation::validate_block_access_list_size;
use ethrex_common::{Bloom, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{EngineType, Store};

use error::T8nError;
use output::Rejection;

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
}

pub fn run(args: T8nArgs) -> Result<(), T8nError> {
    if args.reward > 0 {
        return Err(T8nError::Unsupported(
            "positive block rewards (pre-merge forks) are not supported".to_string(),
        ));
    }
    let inputs = types::read_inputs(&args)?;
    let fork = fork::parse_fork(&args.fork)?;
    let config = fork::chain_config_for(fork, args.chain_id);
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
    // Prefer the environment's real parent hash (`blockHashes[number-1]`)
    // over the fabricated genesis hash: the EIP-2935 system call writes the
    // payload's parent hash into the history contract, so it must match the
    // test's chain. The genesis-state header is aliased under that hash so
    // parent lookups still resolve.
    let genesis_hash = parent_header.hash();
    let parent_hash = block_hash_cache
        .get(&env.current_number.wrapping_sub(1))
        .copied()
        .unwrap_or(genesis_hash);
    if parent_hash != genesis_hash {
        runtime
            .block_on(store.add_block_header(parent_hash, parent_header.clone()))
            .map_err(|e| T8nError::Store(e.to_string()))?;
    }

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
    let (result, build_rejections, build_block_exception) = blockchain
        .build_payload_t8n(
            Block::new(header, body),
            transactions,
            block_hash_cache,
            args.state_test,
        )
        .map_err(|e| T8nError::Build(e.to_string()))?;

    // Map build-time rejection indices (into the converted list) back to
    // input positions and merge with the conversion-time rejections.
    for rejection in build_rejections {
        rejections.push(Rejection {
            index: input_indices[rejection.index],
            error: rejection.error,
        });
    }
    rejections.sort_by_key(|rejection| rejection.index);

    // Block-level validity that only shows post-build: reported as
    // `blockException` so fillers can match expected-invalid blocks.
    let block_exception = build_block_exception.or_else(|| {
        result.block_access_list.as_ref().and_then(|bal| {
            validate_block_access_list_size(&result.payload.header, &config, bal)
                .err()
                .map(|error| error.to_string())
        })
    });

    output::write_outputs(&args, &inputs.alloc, &result, &rejections, block_exception)
}
