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

pub mod state_root;
pub mod types;
