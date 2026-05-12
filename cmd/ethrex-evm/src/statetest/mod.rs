//! # `statetest` subcommand — design notes (Phase 3 spike)
//!
//! ## Phase 3.2 decision: Option B — inline types
//!
//! The `ef_tests-state` crate lives in `tooling/Cargo.toml`, which is a **separate Cargo
//! workspace** from the main `ethrex_2/Cargo.toml`. Depending on it from a main-workspace crate
//! would require either:
//!
//! - Adding a path dependency that crosses workspace boundaries (unsupported by Cargo without
//!   workspace-level `[patch]` gymnastics), or
//! - Publishing the crate — which is not the case here.
//!
//! Additionally, `ef_tests-state` pulls in `revm` (v27) and `simd-json`, neither of which belongs
//! in the main workspace. Therefore **Option B (inline the required types)** is the correct
//! approach: we define the minimal `StateTest` parsing types we need inside this module rather
//! than taking on the transitive dependency.
//!
//! ## Phase 3.1 — exact API call sequence per subtest
//!
//! The call sequence the Phase 4 `statetest` CLI will follow for each (fork, subtest) pair:
//!
//! ```text
//! 1. Parse test JSON → StateTest (types.rs inlined types, serde_json).
//!
//! 2. Build pre_state: FxHashMap<Address, Account> from StateTest::pre.
//!
//! 3. Build and execute the VM:
//!    a. Construct a Genesis from pre_state (accounts → GenesisAccount alloc).
//!    b. let mut store = Store::new("<scratch>", EngineType::InMemory)?;
//!    c. store.add_initial_state(genesis).await?;           // async
//!    d. let block_header = genesis.get_block().header;
//!    e. let vm_db: DynVmDatabase =
//!           Box::new(StoreVmDatabase::new(store.clone(), block_header)?);
//!    f. let mut db = GeneralizedDatabase::new(Arc::new(vm_db));
//!    g. Build Environment + TxKind from the subtest transaction fields.
//!    h. let mut vm = VM::new(env, &mut db);
//!    i. Optionally attach streaming EIP-3155 tracer.
//!    j. vm.execute();
//!
//! 4. Extract state transitions:
//!    let updates = LEVM::get_state_transitions(&mut db)?;
//!    // LEVM::get_state_transitions is a *static function* on LEVM taking &mut GeneralizedDatabase.
//!    // Source: crates/vm/backends/levm/mod.rs:2090
//!
//! 5. Compute post-state root:
//!    let root = compute_post_state_root(&pre_state, &updates)?;
//!    // Internally: applies updates via Store::apply_account_updates_batch (sync, store.rs:1753)
//!    // against the genesis block hash. Returns state_trie_hash from AccountUpdatesList.
//!
//! 6. Emit result:
//!    println!("{}", serde_json::to_string(&PostStateOutput { state_root: root })?);
//! ```
//!
//! ## Async boundary
//!
//! `Store::add_initial_state` is `async` (`store.rs:2110`), but
//! `Store::apply_account_updates_batch` is sync (`store.rs:1753`). Therefore
//! `compute_post_state_root` spins up a single-shot `tokio::runtime::Runtime` internally to drive
//! the async setup, then uses the sync trie update path. The public API remains a plain `fn`.
//!
//! ## Open questions discovered during the spike
//!
//! - None blocking Phase 4. `apply_account_updates_batch` is public and accessible without
//!   any API changes.

pub mod error_map;
pub mod runner;
pub mod state_root;
pub mod types;

use std::path::PathBuf;

/// Arguments for the `statetest` subcommand.
///
/// Flag names match geth's `cmd/evm/statetest` exactly so that goevmlab
/// can invoke this binary as a drop-in replacement.
///
/// Authoritative reference: `go-ethereum/cmd/evm/main.go` flag definitions.
/// - `trace.nomemory` default: `true` (memory disabled by default)
/// - `trace.noreturndata` default: `true` (return data disabled by default)
/// - `trace.nostack` default: `false` (stack enabled by default)
/// - `trace.nostorage` default: `false` (storage enabled by default)
///
/// All boolean flags use `num_args(0..=1)` so they accept both `--flag` and
/// `--flag=true` / `--flag=false` — matching goevmlab's invocation style which
/// passes e.g. `--trace.nomemory=true`.
#[derive(clap::Args, Debug, Clone)]
pub struct StatetestArgs {
    /// Enable EIP-3155 structured-logging trace output on stderr.
    /// Bare boolean (no value accepted); presence ⇒ true, absence ⇒ false.
    /// This is what goevmlab passes: `statetest --trace <path>`.
    #[arg(long = "trace", action = clap::ArgAction::SetTrue)]
    pub trace: bool,

    /// Trace output format.
    /// Only `"json"` is currently supported; other values cause exit(1).
    /// Geth default: `"json"`.
    #[arg(long = "trace.format", default_value = "json")]
    pub trace_format: String,

    /// Include memory in each trace step (opt-in; geth default: disabled).
    #[arg(long = "trace.memory", default_value = "false", num_args(0..=1), require_equals = false, value_parser = parse_bool)]
    pub trace_memory: bool,

    /// Disable memory in trace output (opt-out alias; geth default: true).
    #[arg(long = "trace.nomemory", default_value = "true", num_args(0..=1), require_equals = false, value_parser = parse_bool)]
    pub trace_nomemory: bool,

    /// Disable stack in trace output (geth default: false).
    #[arg(long = "trace.nostack", default_value = "false", num_args(0..=1), require_equals = false, value_parser = parse_bool)]
    pub trace_nostack: bool,

    /// Disable return data in trace output (geth default: true).
    #[arg(long = "trace.noreturndata", default_value = "true", num_args(0..=1), require_equals = false, value_parser = parse_bool)]
    pub trace_noreturndata: bool,

    /// Disable storage capture in trace output (geth default: false).
    #[arg(long = "trace.nostorage", default_value = "false", num_args(0..=1), require_equals = false, value_parser = parse_bool)]
    pub trace_nostorage: bool,

    /// Only run tests for the specified fork (e.g. `Prague`, `Cancun`).
    #[arg(long = "statetest.fork")]
    pub statetest_fork: Option<String>,

    /// Only run the subtest at the given index (0-based).
    #[arg(long = "statetest.index")]
    pub statetest_index: Option<usize>,

    /// Regex filter applied to test names.
    #[arg(long = "run")]
    pub run: Option<String>,

    /// Paths to JSON state-test files or directories.
    /// When empty, paths are read from stdin (batch mode), one per line.
    #[arg()]
    pub paths: Vec<PathBuf>,
}

/// Parses `"true"` / `"false"` (case-insensitive) and bare flag invocations.
///
/// When the flag is specified without a value (e.g. `--trace.nomemory`), clap
/// passes the `default_value` string.  When specified as `--trace.nomemory=true`
/// goevmlab style, this parser handles both forms.
fn parse_bool(s: &str) -> Result<bool, String> {
    match s.to_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        other => Err(format!("expected boolean (true/false), got: {other}")),
    }
}
