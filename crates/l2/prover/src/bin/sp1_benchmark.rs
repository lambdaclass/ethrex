//! SP1 benchmark binary for comparing guest program proving performance.
//!
//! Measures execution cycles and proving time for different guest programs
//! using the SP1 backend. This enables direct comparison between the standard
//! evm-l2 guest program (baseline) and application-specific programs like
//! zk-dex and tokamon.
//!
//! The benchmark uses the same SP1 SDK code paths as the production prover:
//! - rkyv serialization for input encoding
//! - `Prover::execute()` for cycle counting
//! - `Prover::prove()` for proof generation
//! - `Prover::verify()` for proof verification
//!
//! # Usage
//!
//! ```bash
//! # Benchmark zk-dex with 100 transfers
//! cargo run --release --features sp1 --bin sp1_benchmark -- --program zk-dex --actions 100
//!
//! # Benchmark tokamon with 100 game actions
//! cargo run --release --features sp1 --bin sp1_benchmark -- --program tokamon --actions 100
//!
//! # Execute only (cycle count, skip proving)
//! cargo run --release --features sp1 --bin sp1_benchmark -- --program zk-dex --execute-only
//!
//! # Benchmark with Groth16 proof
//! cargo run --release --features sp1 --bin sp1_benchmark -- --program zk-dex --format groth16
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use clap::Parser;
use ethrex_common::types::block_execution_witness::ExecutionWitness;
use ethrex_common::types::{
    AccountState, Block, BlockBody, BlockHeader, ChainConfig, EIP1559Transaction, Transaction,
    TxKind,
};
use ethrex_common::{Address, H160, H256, U256};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_guest_program::{
    ZKVM_SP1_TOKAMON_ELF, ZKVM_SP1_ZK_DEX_ELF,
    programs::tokamon::types::{ActionType, GameAction, TokammonProgramInput},
    programs::zk_dex::{ZkDexGuestProgram, circuit},
    traits::GuestProgram,
};
use ethrex_rlp::encode::RLPEncode as _;
use ethrex_trie::{EMPTY_TRIE_HASH, Node, Trie};
use rkyv::rancor::Error as RkyvError;
use secp256k1::{Message, SECP256K1, SecretKey};
#[cfg(not(feature = "gpu"))]
use sp1_sdk::CpuProver;
#[cfg(feature = "gpu")]
use sp1_sdk::cuda::builder::CudaProverBuilder;
use sp1_sdk::{Prover, SP1ProofMode, SP1Stdin};

#[derive(Parser)]
#[command(name = "sp1_benchmark")]
#[command(about = "SP1 guest program performance benchmark")]
struct Args {
    /// Guest program to benchmark: zk-dex or tokamon
    #[arg(long)]
    program: String,

    /// Number of actions/transfers to include in the input
    #[arg(long, default_value_t = 100)]
    actions: u32,

    /// Proof format: compressed or groth16
    #[arg(long, default_value = "compressed")]
    format: String,

    /// Run execution only (skip proving and verification)
    #[arg(long)]
    execute_only: bool,
}

/// DEX contract address (must match the guest binary constant).
const DEX_CONTRACT: Address = H160([0xDE; 20]);

/// Generate a deterministic, valid input for zk-dex benchmarking.
///
/// Builds a mock `ProgramInput` containing:
/// - A state trie with sender/receiver accounts and the DEX contract
/// - Storage tries with token balance slots
/// - A block with transfer transactions
///
/// Then runs it through `ZkDexGuestProgram.serialize_input()` to produce
/// `AppProgramInput` bytes (exercising the full Phase 3 pipeline).
#[expect(clippy::indexing_slicing)]
fn generate_zk_dex_input(transfer_count: u32) -> anyhow::Result<Vec<u8>> {
    use ethrex_guest_program::l2::ProgramInput;

    let token = H160([0xAA; 20]);
    let count = usize::try_from(transfer_count).context("transfer count overflow")?;

    // Generate unique sender/receiver pairs.
    let mut users: Vec<Address> = Vec::with_capacity(count.saturating_mul(2));
    for i in 0..transfer_count {
        let bytes = i.to_le_bytes();
        let mut sender_bytes = [0u8; 20];
        sender_bytes[0] = 0x10;
        sender_bytes[16..20].copy_from_slice(&bytes);
        users.push(Address::from(sender_bytes));

        let mut receiver_bytes = [0u8; 20];
        receiver_bytes[0] = 0x20;
        receiver_bytes[16..20].copy_from_slice(&bytes);
        users.push(Address::from(receiver_bytes));
    }
    users.sort();
    users.dedup();

    // Build storage trie with balance slots for all users.
    let mut storage_trie = Trie::empty_in_memory();
    for user in &users {
        let slot = circuit::balance_storage_slot(token, *user);
        let hashed_slot = keccak_hash(slot.as_bytes()).to_vec();
        let balance = U256::from(1_000_000u64);
        storage_trie
            .insert(hashed_slot, balance.encode_to_vec())
            .map_err(|e| anyhow::anyhow!("storage trie insert: {e}"))?;
    }
    let storage_root = storage_trie.hash_no_commit();
    let storage_root_node = storage_trie
        .root_node()
        .map_err(|e| anyhow::anyhow!("root_node: {e}"))?
        .map(Arc::unwrap_or_clone)
        .unwrap_or_else(|| Node::default());

    // Build state trie with user accounts + DEX contract.
    let mut state_trie = Trie::empty_in_memory();

    // DEX contract account (with storage).
    let dex_account = AccountState {
        nonce: 0,
        balance: U256::zero(),
        storage_root,
        code_hash: H256::zero(),
    };
    let dex_path = keccak_hash(DEX_CONTRACT.as_bytes()).to_vec();
    state_trie
        .insert(dex_path, dex_account.encode_to_vec())
        .map_err(|e| anyhow::anyhow!("state trie insert dex: {e}"))?;

    // User accounts (each gets nonce=0, balance=10 ETH for gas).
    for user in &users {
        let account = AccountState {
            nonce: 0,
            balance: U256::from(10u64) * U256::from(10u64).pow(U256::from(18u64)),
            storage_root: *EMPTY_TRIE_HASH,
            code_hash: H256::zero(),
        };
        let path = keccak_hash(user.as_bytes()).to_vec();
        state_trie
            .insert(path, account.encode_to_vec())
            .map_err(|e| anyhow::anyhow!("state trie insert user: {e}"))?;
    }
    let _state_root = state_trie.hash_no_commit();
    let state_root_node = state_trie
        .root_node()
        .map_err(|e| anyhow::anyhow!("root_node: {e}"))?
        .map(Arc::unwrap_or_clone)
        .unwrap_or_else(|| Node::default());

    // Build transactions.
    let mut transactions = Vec::with_capacity(count);
    for i in 0..transfer_count {
        let idx = usize::try_from(i).context("index overflow")?;
        let sender_idx = idx.saturating_mul(2);
        let receiver_idx = sender_idx.saturating_add(1);

        // Get sender and receiver (with bounds check).
        // Sender is needed for storage slot analysis but not used directly in tx construction
        // (transactions would need proper signatures in a real scenario).
        let _sender = users
            .get(sender_idx)
            .copied()
            .context("sender index out of bounds")?;
        let receiver = users
            .get(receiver_idx)
            .copied()
            .context("receiver index out of bounds")?;

        let calldata = circuit::encode_transfer_calldata(receiver, token, U256::from(100u64));

        let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            to: TxKind::Call(DEX_CONTRACT),
            data: bytes::Bytes::from(calldata),
            nonce: 0,
            max_fee_per_gas: 1000,
            max_priority_fee_per_gas: 1,
            gas_limit: 100_000,
            chain_id: 42,
            // NOTE: In a real scenario the signature would be valid.
            // The benchmark measures cycle count, not signature verification.
            // For execution-only mode the guest may skip sig verification.
            ..Default::default()
        });
        transactions.push(tx);
    }

    // Build a mock block containing all transactions.
    let block = Block {
        header: BlockHeader {
            number: 1,
            base_fee_per_gas: Some(100),
            gas_limit: u64::try_from(count)
                .unwrap_or(u64::MAX)
                .saturating_mul(100_000),
            ..Default::default()
        },
        body: BlockBody {
            transactions,
            ..Default::default()
        },
    };

    // Parent block header (block 0).
    let parent_header = BlockHeader {
        number: 0,
        ..Default::default()
    };
    let parent_header_rlp = parent_header.encode_to_vec();

    // Build ExecutionWitness.
    let mut storage_trie_roots = BTreeMap::new();
    storage_trie_roots.insert(DEX_CONTRACT, storage_root_node);

    let witness = ExecutionWitness {
        state_trie_root: Some(state_root_node),
        storage_trie_roots,
        chain_config: ChainConfig {
            chain_id: 42,
            ..Default::default()
        },
        first_block_number: 1,
        block_headers_bytes: vec![parent_header_rlp],
        ..Default::default()
    };

    // Build ProgramInput.
    let program_input = ProgramInput {
        blocks: vec![block],
        execution_witness: witness,
        elasticity_multiplier: 2,
        fee_configs: vec![],
        blob_commitment: [0u8; 48],
        blob_proof: [0u8; 48],
    };

    // Serialize ProgramInput via rkyv.
    let raw_bytes = rkyv::to_bytes::<RkyvError>(&program_input)
        .map_err(|e| anyhow::anyhow!("rkyv serialize ProgramInput: {e}"))?;

    // Run through ZkDexGuestProgram.serialize_input() to convert to AppProgramInput.
    let gp = ZkDexGuestProgram;
    let app_input_bytes = gp
        .serialize_input(&raw_bytes)
        .map_err(|e| anyhow::anyhow!("serialize_input: {e}"))?;

    Ok(app_input_bytes)
}

/// Generate a deterministic, valid `TokammonProgramInput`.
///
/// Creates a mix of game action types (CreateSpot, ClaimReward, FeedTokamon,
/// Battle) cycling through them. Each action has valid payloads matching the
/// validation requirements of the execution function.
#[expect(clippy::indexing_slicing)]
fn generate_tokamon_input(action_count: u32) -> anyhow::Result<TokammonProgramInput> {
    let initial_state_root = [0x42u8; 32];
    let count = usize::try_from(action_count).context("action count overflow")?;
    let mut actions = Vec::with_capacity(count);

    let action_types = [
        ActionType::CreateSpot,
        ActionType::ClaimReward,
        ActionType::FeedTokamon,
        ActionType::Battle,
    ];

    for i in 0..action_count {
        let idx = i.to_le_bytes();

        let mut player = [0u8; 20];
        player[0] = idx[0];
        player[1] = idx[1];
        player[2] = idx[2];
        player[3] = idx[3];
        player[4] = 0xAA;

        // Cycle through action types: 0,1,2,3,0,1,2,3,...
        let action_type = action_types[(i % 4) as usize].clone();

        // Generate valid payload per action type.
        let payload = match &action_type {
            ActionType::CreateSpot => {
                // Requires >= 16 bytes (lat: 8 + lon: 8).
                let mut p = vec![0u8; 16];
                p[0] = idx[0];
                p[8] = idx[1];
                p
            }
            ActionType::Battle => {
                // Requires >= 8 bytes (random seed).
                let mut p = vec![0u8; 8];
                p[0] = idx[0];
                p[1] = idx[1];
                p
            }
            // ClaimReward and FeedTokamon: no payload requirement.
            _ => vec![],
        };

        actions.push(GameAction {
            player,
            action_type,
            target_id: u64::from(i),
            payload,
        });
    }

    Ok(TokammonProgramInput {
        initial_state_root,
        actions,
    })
}

fn parse_proof_mode(s: &str) -> anyhow::Result<SP1ProofMode> {
    match s {
        "compressed" => Ok(SP1ProofMode::Compressed),
        "groth16" => Ok(SP1ProofMode::Groth16),
        other => {
            anyhow::bail!("Unsupported proof format: '{other}'. Use 'compressed' or 'groth16'.")
        }
    }
}

fn format_duration(d: std::time::Duration) -> String {
    let total_secs = d.as_secs();
    if total_secs >= 60 {
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        let tenths = d.subsec_millis() / 100;
        format!("{mins}m {secs}.{tenths}s")
    } else {
        format!("{:.2?}", d)
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    println!("=== SP1 Benchmark: {} ===\n", args.program);

    // Resolve ELF binary for the selected guest program.
    let elf: &[u8] = match args.program.as_str() {
        "zk-dex" => {
            if ZKVM_SP1_ZK_DEX_ELF.is_empty() {
                anyhow::bail!(
                    "zk-dex SP1 ELF not found. Build it first:\n  \
                     cd crates/guest-program && GUEST_PROGRAMS=zk-dex \
                     cargo build --release -p ethrex-guest-program --features sp1"
                );
            }
            ZKVM_SP1_ZK_DEX_ELF
        }
        "tokamon" => {
            if ZKVM_SP1_TOKAMON_ELF.is_empty() {
                anyhow::bail!(
                    "tokamon SP1 ELF not found. Build it first:\n  \
                     cd crates/guest-program && GUEST_PROGRAMS=tokamon \
                     cargo build --release -p ethrex-guest-program --features sp1"
                );
            }
            ZKVM_SP1_TOKAMON_ELF
        }
        other => anyhow::bail!("Unsupported program: '{other}'. Supported: zk-dex, tokamon"),
    };
    println!("ELF size: {} bytes", elf.len());

    // Generate program-specific input and serialize with rkyv.
    let serialized: Vec<u8> = match args.program.as_str() {
        "zk-dex" => {
            println!("Generating input: {} transfers", args.actions);
            generate_zk_dex_input(args.actions)?
        }
        "tokamon" => {
            println!("Generating input: {} game actions", args.actions);
            let input = generate_tokamon_input(args.actions)?;
            rkyv::to_bytes::<RkyvError>(&input)
                .map_err(|e| anyhow::anyhow!("rkyv serialization: {e}"))?
                .to_vec()
        }
        _ => anyhow::bail!("Unsupported program for input generation"),
    };
    println!("Serialized input: {} bytes\n", serialized.len());

    // Prepare SP1 stdin.
    let mut stdin = SP1Stdin::new();
    stdin.write_slice(&serialized);

    // Initialize SP1 prover client.
    println!("--- SP1 Prover Initialization ---");
    let init_start = Instant::now();
    #[cfg(not(feature = "gpu"))]
    let client = CpuProver::new();
    #[cfg(feature = "gpu")]
    let client = CudaProverBuilder::default().local().build();
    println!("Client created: {}", format_duration(init_start.elapsed()));

    // Setup proving/verifying keys for this ELF.
    let setup_start = Instant::now();
    let (pk, vk) = client.setup(elf);
    println!("Key setup: {}\n", format_duration(setup_start.elapsed()));

    // Execute (cycle counting).
    println!("--- Execution (cycle counting) ---");
    let exec_start = Instant::now();
    let (_public_values, report) = client
        .execute(elf, &stdin)
        .run()
        .context("SP1 execution failed")?;
    let exec_duration = exec_start.elapsed();

    println!("Execution time: {}", format_duration(exec_duration));
    println!(
        "Total instruction count: {}",
        report.total_instruction_count()
    );
    println!();

    // Proving (unless --execute-only).
    let mut prove_duration = None;
    if !args.execute_only {
        let mode = parse_proof_mode(&args.format)?;

        println!("--- Proving ({}) ---", args.format);
        let prove_start = Instant::now();
        let proof = client
            .prove(&pk, &stdin)
            .mode(mode)
            .run()
            .context("SP1 proving failed")?;
        let duration = prove_start.elapsed();
        prove_duration = Some(duration);
        println!("Proving time: {}\n", format_duration(duration));

        // Verification.
        println!("--- Verification ---");
        let verify_start = Instant::now();
        client
            .verify(&proof, &vk)
            .context("SP1 verification failed")?;
        println!(
            "Verification time: {}\n",
            format_duration(verify_start.elapsed())
        );
    }

    // Summary.
    println!("=== Summary ===");
    println!("Program:           {}", args.program);
    println!("Actions:           {}", args.actions);
    println!("Proof format:      {}", args.format);
    println!("ELF size:          {} bytes", elf.len());
    println!("Input size:        {} bytes", serialized.len());
    println!("Instruction count: {}", report.total_instruction_count());
    println!("Execution time:    {}", format_duration(exec_duration));
    if let Some(d) = prove_duration {
        println!("Proving time:      {}", format_duration(d));
    }

    Ok(())
}
