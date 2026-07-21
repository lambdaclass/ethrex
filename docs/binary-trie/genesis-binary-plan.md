# EIP-7864 Binary-Trie Backend — New Phase: Genesis-from-Binary Full Sync

**Status**: APPROVED (open questions resolved by user 2026-05-04). Slots between Phase 7 (CLI + activation, closed) and the previous Phase 8 (RPC + metrics, now renumbered to Phase 9).

**Numbering**: this is **Phase 8**. Existing Phases 8/9/10 in `plan.md` renumber to 9/10/11. Handoff filename: `docs/binary-trie/phases/phase-8-handoff.md`.

---

## 1. Executive Summary

This phase adds a **new entry path into binary-trie mode**: a node launched with `--binary-from-genesis` constructs the genesis state directly inside a `BinaryBackend` and full-syncs every subsequent block into the binary overlay, **without ever using MPT for state**. No transition, no overlay, no MPT freeze. The path is purely additive — the existing transition flow (Phases 0–7) is unchanged. This phase reverses `design-decisions.md` §11 ("No genesis-from-binary") and updates `overview.md`'s "What this feature is NOT" line about snap-sync targets only by adding context — the current bullet stays since this phase still does not enable snap-sync into binary, only full-sync from genesis. Existing test machinery (EF state-test + EF blockchain-test + LEVM unit tests + per-block receipts/gas/tx-root verification) provides cross-validation; no dual-node diff is built.

## 2. Goals and Non-Goals

### Goals (in scope)
- New CLI flag `--binary-from-genesis` (boolean), mutually exclusive with `--binary-transition` and incompatible with `--syncmode=snap`.
- New `Store::new_binary_only(...)` startup path that opens a fresh DB with format byte `1` (`Binary`) and writes the genesis state into a `BinaryBackend` directly from `Genesis::alloc`.
- Reuse the existing post-Phase-5 `BinaryBackend` / `BinaryMerkleizer` / `binary_wiring.rs` plumbing without modification. The genesis dispatch is the only new logic.
- Reuse the existing post-Phase-7 state-root-skip gate (`BackendKind != Mpt` → skip `validate_state_root`). Whether that gate already exists in `blockchain.rs` is verified in Task X.0 below; if missing, it is added in Task X.1 as a precondition to this phase, since Phase 7 documents it as already gated and this phase depends on it.
- All non-state-root header verifications remain on: `transactions_root`, `withdrawals_root`, `receipts_root`, `gas_used`, `logs_bloom`, EIP-4895 / 4844 / 7002 / 7251 system contract execution. None of these depend on the trie format.
- Restart path: opening an existing DB with format byte `1` and `--binary-from-genesis` reconstructs `StateBackend::Binary(BinaryBackend)` (no overlay, no transition metadata reads).
- Genesis allocation handles all hardfork system contracts (beacon roots `0xBEAC0...02`, history storage `0x000F3DF6...`, deposit contract, withdrawal contract, EIP-7002 / EIP-7251 contracts as applicable per chain config) identically to MPT genesis.
- Genesis-alloc accounts that carry bytecode are dual-written: code chunks go through the binary trie at `(stem, CODE_OFFSET + i)` per EIP-7864, and the raw `code_hash → bytecode` is written to `AccountCodes` so EXTCODECOPY/CODECOPY work without chunk reassembly.
- Tests: integration test that boots a fresh in-memory store with `--binary-from-genesis`, applies N synthetic blocks, asserts (a) backend kind is `Binary`, (b) state-root field in stored header equals the binary root computed by the backend, (c) reads round-trip through restart, (d) binary FKV is populated inline. EF state-test smoke run on a small subset proves cross-validation is wired.

### Non-Goals (explicit)
- **No transition** between MPT and binary. `--binary-from-genesis` is mutually exclusive with `--binary-transition`.
- **No snap sync** into binary. Snap sync stays MPT-only (`design-decisions.md` §1, `overview.md` lines 38–45). Combining `--binary-from-genesis` with `--syncmode=snap` is an error at startup.
- **No state-root verification post-execution**. Inherited from Phase 7's already-locked gate. Receipts root, gas used, transactions root, withdrawals root, logs bloom remain verified.
- **No witness generation**. Inherited from Phase 7's already-locked gate (`BackendKind != Mpt` returns error).
- **No genesis-state migration tool**. Genesis comes from the standard `Genesis` struct in `ethrex-common`.
- **No dual-node diff harness**. User explicitly rejected this; existing EF tests are the cross-check.
- **No RPC additions in this phase**. `eth_getBinaryProof` lands in (renumbered) Phase 9 / current Phase 8.
- **No metrics additions in this phase**. (Renumbered) Phase 9 / current Phase 8 owns metrics. The `binary_trie_switch_activation_timestamp` gauge is irrelevant in genesis-binary mode (no switch occurs); current Phase 8 must be updated to handle that case but that update is not part of this phase.
- **No mainnet operation**. Block hashes computed by a genesis-binary node will not match mainnet block hashes (state_root field carries the binary root, not the canonical MPT root). This is research-only; a genesis-binary node CANNOT follow public mainnet because peers will reject blocks whose computed hash diverges from the canonical chain. Documented in (renumbered) Phase 11 / current Phase 10 polish doc.
- **No L2 support**. `--binary-from-genesis` is L1-only. L2 startup paths in `cmd/ethrex/l2/` are untouched.
- **No genesis-from-binary support in the L2 deployer** (`cmd/ethrex/l2/deployer.rs` continues to call `BackendKind::Mpt` for genesis-root computation).

## 3. Locked Decisions

| # | Decision | Choice | Source |
|---|---|---|---|
| 1 | CLI flag name | `--binary-from-genesis` (boolean, `ArgAction::SetTrue`) | User |
| 2 | Mutual exclusion with `--binary-transition` | Setting both is a startup error | User |
| 3 | Sync mode constraint | `--syncmode=snap` + `--binary-from-genesis` is a startup error; only `--syncmode=full` is permitted (the dev shortcut `opts.dev` flips syncmode to Full and is therefore allowed) | User |
| 4 | State-root verification post-execution | Disabled (reuses Phase 7 gate `BackendKind != Mpt`); no new gate added | User |
| 5 | Receipts / gas / tx-root / withdrawals root / logs bloom verification | Remain on (already verified by `execute_block_pipeline` regardless of state backend) | User |
| 6 | Cross-validation harness | LEVM unit tests + per-block always-on header field verification (receipts root, transactions root, withdrawals root, gas, logs bloom). **No dual-node diff. No EF tests against the binary backend** (user explicitly excluded — EF vectors assume MPT post-state roots). | User |
| 7 | Witness generation | Disabled (reuses Phase 7 gate `BackendKind != Mpt` returning error) | User |
| 8 | Genesis state-root field in genesis block header | **Genesis block 0 header `state_root` = canonical MPT genesis root** (computed via the existing `genesis_root()` in `crates/common/trie/genesis.rs`). The binary genesis root is computed internally for bookkeeping (logged at startup, exposable via RPC in a later phase) but **never overwrites the header field**. From block 1 onward, peer-supplied headers are stored verbatim; we compute a binary state internally and do not compare it against the header (Phase 7's `BackendKind != Mpt` gate). Same trust pattern as transition mode: trust peer headers, compute binary internally, never compare. **Network-compatible**: genesis block hash matches canonical mainnet/testnet, so peers accept us. | User |
| 9 | Mutex-exclusivity enforcement location | clap's `conflicts_with = "binary_transition"` on the new flag, **plus** a manual check in `init_l1` (`initializers.rs`) for the `--syncmode=snap` combination (clap's `conflicts_with` doesn't compose cleanly with arg-with-default values like `syncmode`). | User |
| 10 | Pre-existing-DB behavior with `--binary-from-genesis` | Format byte `0` (Mpt) on disk + flag → startup error, message `"--binary-from-genesis requires a fresh datadir; existing DB is in MPT mode. Run 'ethrex removedb' first."`. Format byte `1` (Binary) + flag → normal restart. Format byte `2` (Transition) + flag → startup error, mismatched mode. Operator must `ethrex removedb` to start fresh. | User |
| 11 | Format byte | `1` (`Binary`); already reserved per Phase 0/1; `Store::from_backend` already handles it | Locked, plan §3 |
| 12 | Hardfork system contracts at genesis | Written via the same `Genesis::alloc` walk used for MPT genesis. All accounts in alloc are inserted using the same `BinaryBackend::update_accounts` + `update_storage` + dual-write code path used during normal block execution. No special-case logic. | Locked |
| 13 | Code chunking + dual-write at genesis | Genesis-alloc accounts with `code` field run through the same code-chunking path as post-block deploys. Dual-write to `AccountCodes` happens in the same commit. Reuses Phase 3 dual-write logic. | Locked |
| 14 | Network compatibility | Genesis-binary mode emits the **canonical MPT genesis state_root** in block 0 (decision #8), so the genesis block hash matches the canonical chain. **A genesis-binary node CAN follow public mainnet/testnets via p2p.** Block 1+ headers come from peers verbatim; the node stores them as-is and computes a parallel binary state internally (no comparison against the canonical state_root field — same as transition mode). | User, derived from #8 |

## 4. Crate / Module Diff

### New
- `crates/storage/binary_genesis.rs` — `Store::add_initial_state_binary(genesis: Genesis)` analog to `add_initial_state` in `mpt_wiring.rs` line 875. Builds the binary genesis block + state. ~150 lines.
- `test/tests/binary_from_genesis.rs` — integration tests (Task X.10–X.13).

### Modified
- `crates/storage/state_backend.rs` — `compute_genesis_root` and `compute_genesis_block` for `BackendKind::Binary` switch from `panic!` to a real implementation. `BackendKind::Transition` stays `panic!` (genesis-from-transition is still impossible — the entry path is via post-MPT-sync activation).
- `crates/storage/mpt_wiring.rs` — extract a small helper `genesis_block_with_root(genesis: &Genesis, state_root: H256) -> Block` that takes a state root parameter (currently `genesis_block` in `crates/common/trie/genesis.rs` line 22 hard-codes the MPT root via `genesis_root(genesis)`). Used by both the MPT and the binary genesis dispatch.
- `crates/common/trie/genesis.rs` — add `pub fn genesis_block_with_root(genesis: &Genesis, state_root: H256) -> Block` alongside the existing `genesis_block`. Existing `genesis_block` becomes a thin wrapper: `genesis_block_with_root(genesis, genesis_root(genesis))`. Existing callers untouched.
- `crates/storage/store.rs` — extend `Store::add_initial_state` (currently MPT-only) to dispatch on the store's backend kind. New private method `add_initial_state_binary` (in `binary_genesis.rs`) is called when `self.backend_kind() == BackendKind::Binary`. `BackendKind::Transition` remains an error here (transition entry is always via Mpt → activator → restart, never `add_initial_state`).
- `cmd/ethrex/cli.rs` — add `--binary-from-genesis` flag (mirrors the existing `--binary-transition` shape at line 370–377). Add `Default::default()` field at line ~462. Add `conflicts_with = "binary_transition"`.
- `cmd/ethrex/initializers.rs` — extend the `backend_kind` selection at line 519 to handle the new flag. Add the `--syncmode=snap` validation. Wire `BackendKind::Binary` into `open_store_with_backend_kind` (already accepts the kind via parameter, only needs to be reached). Skip `TransitionActivator` instantiation when `--binary-from-genesis` is set (the activator is irrelevant; format byte is `1` from genesis).
- `crates/blockchain/blockchain.rs` — IF Task X.0 verifies that `validate_state_root` is not yet gated by `BackendKind` (current Phase 7 doc says it is, but the in-tree code at line 935 / 1442 calls it unconditionally), gate both call sites on `self.storage.backend_kind() != BackendKind::Mpt` to skip. This is the single change that lets both transition mode and genesis-binary mode coexist. If Phase 7 already adds this gate, this task becomes a no-op.
- `docs/binary-trie/design-decisions.md` — §11 ("No genesis-from-binary") gets a follow-up paragraph: "**Updated**: genesis-from-binary IS supported as of (renumbered) Phase 8 / current Phase 7.5, behind `--binary-from-genesis`. The original rationale (entry only via transition) is preserved for the transition flow; the genesis path is purely additive." The doc edit lands in the (renumbered) Phase 10 / current Phase 9 docs phase, not in this phase. This phase only adds a one-line `// See genesis-binary-plan.md for the override` comment in the place that currently `panic!`s.
- `docs/binary-trie/overview.md` — line 88 says `"Attempting to start a fresh DB with '--binary-only' is an error."` That sentence becomes incorrect after this phase. Update in (renumbered) Phase 10 / current Phase 9 docs phase. Out of scope for this phase.
- `docs/binary-trie/plan.md` — locked-decisions table row "Genesis-binary: Unsupported" is overturned. Update lands in (renumbered) Phase 10 docs phase.

### Untouched
- `crates/common/binary-trie/` — the binary trie crate. All needed primitives exist post-Phase-5 (`BinaryTrie::new()`, `BinaryTrieState::new()`, `BinaryBackend::new()`, `BinaryBackend::new_with_db`, `BinaryMerkleizer`).
- `crates/storage/binary_wiring.rs` — already supports an empty trie + writes against it.
- `crates/storage/transition_wiring.rs` — transition flow is fully orthogonal to genesis-binary.
- `crates/networking/rpc/` — no RPC changes.
- `crates/blockchain/transition_activator.rs` — never instantiated when `--binary-from-genesis` is set.
- All of `crates/l2/`.
- `crates/networking/p2p/` — no sync-mode changes (snap stays MPT-only).

## 5. Core Types

**No new variants, no new constants.**

`BackendKind::Binary` already exists from Phase 0. Format byte `1` already exists from Phase 1. `StateBackend::Binary(BinaryBackend)` already exists from Phase 5. `BinaryBackend::new()` and `BinaryBackend::new_with_db(provider, code_reader)` already exist. `BinaryMerkleizer::new(parent_root, ...)` accepts `EMPTY_BINARY_ROOT` (`H256([0u8; 32])`) as parent.

The only "new" surface is the `add_initial_state_binary` method and the public `genesis_block_with_root` helper. Neither introduces a new type.

## 6. Phase Task Breakdown

Numbering: tasks use `X.N` where `X` will become the chosen phase number (8 or 7.5) at merge time. Cited here as `X` for clarity.

### Tasks

- [ ] **Task X.0 — Audit current state-root-validation gating.** (Low)
  Verify whether `crates/blockchain/blockchain.rs` lines 935 and 1442 (`validate_state_root(&block.header, merkle_output.root)`) are already gated by `self.storage.backend_kind()`. If yes, document the gate location in this plan and skip Task X.1. If no, Task X.1 is mandatory. Output: a one-line note added to this file's "Resolved Audit" section at the bottom.

- [ ] **Task X.1 — Gate `validate_state_root` on `BackendKind` (only if Task X.0 finds it ungated).** (Medium)
  In `crates/blockchain/blockchain.rs`:
  - At line 935 (in `store_block`), wrap `validate_state_root(&block.header, merkle_output.root)?;` in `if self.storage.backend_kind() == BackendKind::Mpt { ... }`. The `Binary` and `Transition` arms compute the root for storage but do not compare it against the header.
  - At line 1442 (in the batch-store path), apply the same gate.
  - Add `Store::backend_kind(&self) -> BackendKind` if not already present. (Phase 8's Task 8.4 mentions adding it; if already there, reuse.)
  - Add a single-line code comment at each gated call site: `// State-root validation is MPT-specific. Binary / Transition modes compute the root but do not compare it against the header — see docs/binary-trie/design-decisions.md §9.`
  - Files: `crates/blockchain/blockchain.rs`, `crates/storage/store.rs`.
  Acceptance: existing MPT tests pass unchanged; a new unit test in `crates/blockchain/` constructs a `Store` with `BackendKind::Binary` and asserts `store_block` does not call into the state-root comparison branch (use a counter or instrument).

- [ ] **Task X.2 — Add `genesis_block_with_root` helper.** (Low)
  In `crates/common/trie/genesis.rs`:
  - Add `pub fn genesis_block_with_root(genesis: &Genesis, state_root: H256) -> Block`. Lift the body of the current `genesis_block` (line 22) into this function with `state_root` as the parameter. Existing `genesis_block` becomes `pub fn genesis_block(genesis: &Genesis) -> Block { genesis_block_with_root(genesis, genesis_root(genesis)) }`.
  - All existing callers of `genesis_block` keep working unmodified.
  Acceptance: `cargo test -p ethrex-trie` passes unchanged; `cargo test --workspace` passes.

- [ ] **Task X.3 — Implement `binary_genesis_root(genesis: &Genesis) -> Result<H256, StateError>`.** (Medium)
  In a new module `crates/storage/binary_genesis.rs`:
  - Construct an in-memory `BinaryBackend::new()` (no DB; `new()` uses `BinaryTrieState::new()`).
  - Walk `genesis.alloc` accounts. For each `(addr, account)`:
    - Build an `AccountInfo { nonce, balance, code_hash: keccak(code), code_size: code.len() }`.
    - Call `backend.update_accounts(&[addr], &[AccountMut { account: Some(info), code: Some(CodeMut { code: Some(code) }) }])`. This goes through the existing post-Phase-5 dual-write code-chunking path.
    - For each storage slot in `account.storage` (typed `HashMap<H256, U256>` per `Genesis`): build the slots vec and call `backend.update_storage(addr, &slots)`.
  - Call `backend.commit()` (consumes self) to produce a `MerkleOutput { root, .. }`. Return `Ok(merkle_output.root)`.
  - Storage of the resulting node diffs is the `add_initial_state_binary`'s job (Task X.4); this function is **pure** (no DB writes), used by `compute_genesis_root` and by external tooling.
  Acceptance: pure function returns the same root for the same `Genesis` regardless of the order of alloc iteration (insertion order is determined by `Genesis::alloc`'s map type — confirm it's a `BTreeMap` or sort before iterating; if it's `HashMap`, sort by address).

- [ ] **Task X.4 — Implement `Store::add_initial_state_binary(genesis: Genesis)`.** (High)
  In `crates/storage/binary_genesis.rs` (`impl Store`):
  - Mirror `add_initial_state` (line 875 of `mpt_wiring.rs`). Steps:
    1. `set_chain_config(&genesis.config).await?;`
    2. Build the genesis state via the new `Merkleizer::Binary(BinaryMerkleizer::new(EMPTY_BINARY_ROOT, false, provider))`. Feed alloc as a single `Vec<AccountUpdate>` (one per alloc account, with `info`, `code`, `added_storage` populated). Call `finalize` to get `MerkleOutput { root, node_updates, code_updates, fkv_entries (in NodeUpdates::Binary) }`.
    3. `genesis_state_root = merkle_output.root` (the binary root, NOT the MPT root).
    4. `let genesis_block = genesis_block_with_root(&genesis, genesis_state_root);` (Task X.2 helper).
    5. Latest-block-header bookkeeping: same as MPT path (lines 887–892).
    6. Header existence check: same as MPT (lines 895–910). On mismatch: same `IncompatibleChainConfig` error (the operator's chain-config / alloc combination doesn't match what's on disk; same semantics).
    7. `self.add_block_header(genesis_hash, genesis_block.header.clone()).await?` (line 907).
    8. Persist the `node_updates` + `code_updates` via the existing `store_block_updates` path. Use `UpdateBatch { node_updates, code_updates, blocks: vec![genesis_block.clone()], receipts: vec![(genesis_hash, vec![])], batch_mode: false }`. This routes through `apply_trie_updates(NodeUpdates::Binary { ... })` which Phase 6 already wired for both `Binary` and `Transition` kinds.
    9. `self.update_earliest_block_number(0).await?;`
    10. `self.forkchoice_update(vec![], 0, genesis_hash, None, None).await?;`
  - Files: `crates/storage/binary_genesis.rs`, `crates/storage/store.rs` (add module), `crates/storage/mpt_wiring.rs` (extract dispatch from `add_initial_state` to a thin wrapper that matches on backend kind).
  Acceptance: covered by Task X.10 integration test.

- [ ] **Task X.5 — Wire genesis dispatch in `state_backend.rs`.** (Low)
  In `crates/storage/state_backend.rs`:
  - `compute_genesis_root(BackendKind::Binary, genesis)` → call `binary_genesis::binary_genesis_root(genesis)` and unwrap into the `H256`. Document: "Pure; uses an in-memory `BinaryBackend`."
  - `compute_genesis_block(BackendKind::Binary, genesis)` → `let root = binary_genesis_root(genesis)?; Ok(genesis_block_with_root(genesis, root))`.
  - `BackendKind::Transition` arms remain `panic!` with the existing message — there is no genesis-from-transition. Add a comment: `// Genesis-from-binary is supported via BackendKind::Binary; transition entry is post-MPT-sync only.`
  - The function signature changes from `-> H256` / `-> Block` to `-> Result<H256, StateError>` / `-> Result<Block, StateError>` to surface allocation walk errors. Update the 4 existing call sites in `cmd/ethrex/cli.rs:665`, `cmd/ethrex/utils.rs:200`, `cmd/ethrex/l2/deployer.rs:1211`, `cmd/ethrex/l2/deployer.rs:1329` to handle the `Result`.
  Acceptance: workspace builds; existing MPT tests pass.

- [ ] **Task X.6 — Add `--binary-from-genesis` CLI flag.** (Low)
  In `cmd/ethrex/cli.rs`:
  - After the existing `--binary-transition` block (line 370–377), add:
    ```
    #[arg(
        long = "binary-from-genesis",
        action = ArgAction::SetTrue,
        help = "Full-sync from genesis directly into binary trie mode (research; standalone or custom-chain only — produces non-canonical block hashes, cannot follow public mainnet).",
        help_heading = "Node options",
        env = "ETHREX_BINARY_FROM_GENESIS",
        conflicts_with = "binary_transition",
    )]
    pub binary_from_genesis: bool,
    ```
  - Add `binary_from_genesis: false` to both `default_l1` (line 382) and `Default::default()` (line 462).
  Acceptance: `ethrex --help` shows the new flag; `ethrex --binary-from-genesis --binary-transition` errors with clap's standard conflicts-with message.

- [ ] **Task X.7 — Validate flag combinations in `init_l1`.** (Medium)
  In `cmd/ethrex/initializers.rs` `init_l1` (around line 500):
  - Immediately after `let genesis = network.get_genesis()?;` (line 510), add:
    ```
    if opts.binary_from_genesis && opts.syncmode == SyncMode::Snap && !opts.dev {
        return Err(eyre::eyre!("--binary-from-genesis requires --syncmode=full (snap sync into binary trie is not supported)"));
    }
    ```
  - Extend the `backend_kind` selection (lines 519–529) to handle `binary_from_genesis`:
    ```
    let backend_kind = if opts.binary_from_genesis {
        match peek_backend_format_byte(&datadir) {
            Some(0) => return Err(eyre::eyre!("--binary-from-genesis requires a fresh datadir; existing DB is in MPT mode (run `ethrex removedb` to reset)")),
            Some(2) => return Err(eyre::eyre!("--binary-from-genesis incompatible with a transitioned DB (format byte 2)")),
            Some(1) | None => BackendKind::Binary,
            Some(other) => return Err(eyre::eyre!("Unknown state backend format byte: {other}")),
        }
    } else if opts.binary_transition {
        // ... existing logic unchanged ...
    } else {
        BackendKind::Mpt
    };
    ```
  - At line 618 (`if opts.binary_transition && backend_kind == BackendKind::Mpt`), the activator instantiation already gates on `binary_transition`; add an explicit `&& !opts.binary_from_genesis` even though clap forbids the combination, as defense-in-depth. Document with a comment.
  Acceptance: covered by Task X.10 unit tests + manual smoke (Task X.13).

- [ ] **Task X.8 — Wire `add_initial_state` dispatch on backend kind.** (Medium)
  In `crates/storage/mpt_wiring.rs` line 875 OR `store.rs`:
  - Convert `add_initial_state` into a dispatcher:
    ```
    pub async fn add_initial_state(&mut self, genesis: Genesis) -> Result<(), StoreError> {
        match self.backend_kind() {
            BackendKind::Mpt => self.add_initial_state_mpt(genesis).await,
            BackendKind::Binary => self.add_initial_state_binary(genesis).await,
            BackendKind::Transition => Err(StoreError::Custom(
                "transition entry is via post-MPT-sync activation, not initial state".into(),
            )),
        }
    }
    ```
  - Rename the current body of `add_initial_state` (lines 875–925) to `add_initial_state_mpt`. No semantic changes.
  Acceptance: existing MPT tests + Task X.10.

- [ ] **Task X.9 — Restart path: open existing format-byte-1 DB.** (Medium)
  In `crates/storage/store.rs` `Store::from_backend`:
  - Format byte `1` (`Binary`) is already handled in the post-Phase-5 code (the user mentioned: `Store::from_backend already accepts it`). Confirm by reading the existing dispatch and adding a unit test if missing.
  - The restart construction path: `StateBackend::new_binary_with_db(provider, code_reader)` where `provider` comes from `make_binary_trie_provider()` (already exists in `binary_wiring.rs`). The trie's `META_ROOT_HASH` is read from `BINARY_TRIE_NODES`; the in-memory `BinaryBackend::from_state(state, code_reader)` reconstructs the live state.
  - `frozen_mpt_root` is **not** read on this path (no overlay). No transition metadata reads.
  - Add explicit log line on restart: `info!("Opening DB in Binary (genesis-from-binary) mode at root {root:?}.");`
  Acceptance: Task X.11 restart test.

- [ ] **Checkpoint: Verify Phase X pre-test tasks (X.0–X.9) complete.** (Low)
  Confirm: state-root gate is in place; helper `genesis_block_with_root` exists; `binary_genesis_root` produces a stable root; `add_initial_state_binary` writes a complete genesis to disk; CLI flag parses; mutual exclusion enforced; restart path works. List each task and its status. Do not proceed until all are done.

- [ ] **Task X.10 — Integration test `binary_from_genesis_smoke`.** (Medium)
  In `test/tests/binary_from_genesis.rs`:
  - Open an in-memory `Store` via `Store::new(":memory:", EngineType::InMemory, BackendKind::Binary)`.
  - Construct a `Genesis` with chain config matching mainnet's Cancun (or whichever fork is the latest the binary path will exercise — pick the one with the most system contracts to maximize coverage).
  - Call `store.add_initial_state(genesis.clone()).await?`.
  - Assert: `store.backend_kind() == BackendKind::Binary`. Genesis block stored. `store.get_block_header(0)?.unwrap().state_root` equals `binary_genesis_root(&genesis)?`.
  - For each alloc account: `store.account_info_at_block(0, addr)?` returns the expected `AccountInfo`.
  - For each alloc account with code: `store.get_account_code(code_hash)?` returns the bytecode (dual-write check).
  - `BINARY_FLATKEYVALUE` table has at least one entry per alloc account (sub-index 0 BASIC_DATA at minimum).
  Acceptance: test passes against in-memory + RocksDB backend.

- [ ] **Task X.11 — Integration test `binary_from_genesis_restart`.** (Medium)
  - Open store with `BackendKind::Binary`, write genesis, close.
  - Reopen the same datadir. Assert: format byte on disk is `1`. `Store::from_backend` reconstructs `StateBackend::Binary`. All alloc accounts are still readable. State root of the genesis block matches a freshly computed `binary_genesis_root`.
  - This test must run on RocksDB (not InMemory) because the restart semantics involve persistent storage. Use `tempfile::TempDir`.
  Acceptance: explicit assertion on `peek_backend_format_byte`; explicit re-read of all alloc accounts.

- [ ] **Task X.12 — Integration test `binary_from_genesis_apply_blocks`.** (High)
  - Set up genesis-binary store as in X.10.
  - Apply 3 synthetic blocks via `Blockchain::add_block`. Each block contains 1–3 transactions touching alloc accounts and creating new ones. Use the same fixture-block helpers used by Phase 6/7 transition tests.
  - After each block: assert backend kind is still `Binary`; `validate_state_root` is NOT called (use the Task X.1 instrumentation if present, or assert that a deliberately wrong `state_root` field in the block header does NOT cause `add_block` to fail — the gate proves itself). Receipts root, gas used, transactions root **are** checked (mutate one of them in a copy of the block; assert `add_block` fails for that copy).
  - Read modified accounts post-block: values match expected.
  Acceptance: 3 blocks applied; reads round-trip; non-state-root header fields verified; state-root field is computed-but-not-verified.

- [ ] **Task X.13 — CLI smoke test `binary_from_genesis_cli_validation`.** (Low)
  Unit test (not full subprocess) that exercises the `init_l1` validation logic with synthetic `Options`:
  - `binary_from_genesis=true` + `binary_transition=true` → clap errors at parse (this is a clap-level test using `Options::try_parse_from`).
  - `binary_from_genesis=true` + `syncmode=Snap` + `dev=false` → `init_l1` returns the "requires --syncmode=full" error.
  - `binary_from_genesis=true` + `syncmode=Snap` + `dev=true` → allowed (dev flips syncmode to Full internally).
  - `binary_from_genesis=true` + existing format-byte-0 DB → `init_l1` returns the "fresh datadir" error.
  - `binary_from_genesis=true` + existing format-byte-2 DB → `init_l1` returns the "incompatible with transitioned DB" error.
  Acceptance: 5 sub-cases, each named, each independently asserting on the error message.

- [ ] **Task X.14 — Update user-facing CLI documentation.** (Low)
  Add `--binary-from-genesis` to the operator-facing docs alongside the existing `--binary-transition` row:
  - `docs/CLI.md` — describe the flag, its constraints (mutex with `--binary-transition`, requires `--syncmode=full`, requires fresh datadir), and the link to `docs/binary-trie/operational.md`.
  - `docs/l1/running/startup.md` — add an example invocation for genesis-binary mode (alongside the existing snap-sync examples) for at least Hoodi.
  - `docs/binary-trie/operational.md` — operator-facing section: what the mode does, what the trust model is (trust peer headers; compute binary internally; no post-block state-root comparison), what the implications are (block 1+ binary state is internal-only; no RPC support yet until renumbered Phase 9). State explicitly that **the genesis block 0 header carries the canonical MPT state_root** so peer compatibility is preserved.
  Acceptance: the three doc files compile (markdown lints if any), and `--binary-from-genesis --help` matches the description in `docs/CLI.md`.

- [ ] **Checkpoint: Verify Phase X test tasks (X.10–X.14) complete.** (Low)
  All 5 integration / smoke tests pass on both InMemory and (where applicable) RocksDB. CLI docs updated. List each test by its function name and pass/fail status.

- [ ] **Task X.15 — Documentation stub.** (Low)
  Add a single section to this file (`docs/binary-trie/genesis-binary-plan.md`) titled "Resolved Audit" containing:
  - Outcome of Task X.0 (gate present or absent).
  - The exact location in `blockchain.rs` where state-root validation is gated post-this-phase.
  - Whether `Genesis::alloc` iterates deterministically (BTreeMap vs sorted Vec).
  No edits to `plan.md`, `overview.md`, or `design-decisions.md` in this phase — those land in (renumbered) Phase 10 / current Phase 9.

- [ ] **Final Audit** — Re-read the entire plan. For each task X.0 through X.15, verify the implementation exists in the codebase via `git diff main...HEAD -- <file>` and `rg`. List any gaps. All gaps must be resolved before reporting completion. Confirm no `TODO`/`FIXME`/`todo!()`/`unimplemented!()` introduced in this phase's diff.

## 7. Tests That Must Pass at Checkpoint

| Test name | File | Property proven |
|---|---|---|
| `binary_from_genesis_smoke` | `test/tests/binary_from_genesis.rs` | Genesis state writes through binary backend; alloc accounts readable; code dual-written; FKV populated inline |
| `binary_from_genesis_restart` | `test/tests/binary_from_genesis.rs` | Format byte 1 persists across close/reopen; reads identical post-restart |
| `binary_from_genesis_apply_blocks` | `test/tests/binary_from_genesis.rs` | 3-block sequence applies; non-state-root header fields verified; state-root field not verified |
| `binary_from_genesis_cli_validation` | `cmd/ethrex/tests/cli_validation.rs` (or co-located unit) | Mutual exclusion + syncmode-snap rejection + DB-format-byte preconditions all enforced |
| `binary_from_genesis_pure_root` | `crates/storage/binary_genesis.rs` (`#[cfg(test)] mod tests`) | `binary_genesis_root` is pure and deterministic across re-invocations |
| `binary_from_genesis_dual_write_codepath` | `test/tests/binary_from_genesis.rs` | Genesis-alloc account with `code` produces an entry in `AccountCodes` AND in `BINARY_TRIE_NODES` (chunk leaves at CODE_OFFSET range) |
| `state_root_gate_binary_kind` (only if Task X.1 ran) | `crates/blockchain/blockchain.rs` (`#[cfg(test)]`) | `validate_state_root` is not called when `backend_kind == BackendKind::Binary` |
| `cargo test -p ethrex-trie` | existing | Regression: `genesis_block` / `genesis_root` unchanged for MPT |
| `cargo test -p ethrex-storage` | existing | Regression: all post-Phase-6 storage tests pass |
| `--binary-from-genesis --help` matches `docs/CLI.md` | docs | Task X.14 — operator-facing CLI doc parity |

## 8. Known Hazards

1. **Genesis allocation order non-determinism.** `Genesis::alloc` is typically a `HashMap<Address, GenesisAccount>` after deserialization. Iteration order is then RNG-seeded per process start. The binary trie's commit produces a deterministic root regardless of insertion order (the trie is a pure function of its key/value set), but applies internal `node_id` allocation in insertion order, which affects the on-disk node layout — NOT the root, but observable via the `META_NEXT_ID` counter. **Mitigation**: in `binary_genesis_root` and `add_initial_state_binary`, sort alloc by address before iterating. This is a deterministic-output guarantee, not a correctness guarantee, but matters for reproducible debugging and CI-stable hashes.

2. **Genesis allocation may exceed `BinaryTrieLayerCache` thresholds.** The cache has a 128-layer commit threshold (`design-decisions.md`). Genesis alloc can have hundreds of accounts on chains like Sepolia/Hoodi (system contracts + dev accounts + premine). All written in a single "block 0" commit. Layer-cache pressure should be fine because it's one block (one layer), but stem count can be 100+. **Mitigation**: explicit unit test that constructs a 200-account genesis and asserts `add_initial_state_binary` succeeds without hitting cache eviction or worker queue overflow. Add to Task X.10 as a sub-case.

3. **First-block apply may need different code-chunk handling than steady-state apply.** Steady-state apply has a parent state to read from; genesis apply writes from empty. The Phase 5 code-chunk path goes through `BinaryMerkleizer::feed_updates(Vec<AccountUpdate>)` which already handles "create from nothing" (it's the universal insert path). **Hazard verification**: Task X.10 must include at least one alloc account with `code.len() > 32 * 2 = 64` bytes so that we exercise multi-chunk insertion at genesis. Use the EIP-7002 system contract (typically ~5KB) as the test fixture.

4. **`Genesis::alloc` may include accounts with `nonce > 0` and `balance > 0` and `code.is_empty()` (EOA premines)**. The `BASIC_DATA` packed-leaf format requires a specific encoding. If the genesis path hits a code path that diverges from the steady-state `update_accounts` path (e.g., uses a "fresh insert" optimization that skips the `code_size` field), the resulting binary root will diverge from what an equivalent steady-state apply would produce. **Mitigation**: explicit assert in Task X.10's `binary_from_genesis_smoke` that running `binary_genesis_root(&g)` produces the same root as: (a) starting from `EMPTY_BINARY_ROOT`, (b) feeding alloc as a Vec<AccountUpdate> through a `BinaryMerkleizer`, (c) finalizing. Both paths must produce identical roots; if they don't, one of them has a bug.

5. **Hardfork system-contract `code_size` parsing.** Some system contracts (`0xBEAC0...`) have specific bytecode lengths; if the binary trie packs `code_size` into BASIC_DATA differently from MPT, post-deploy reads via `EXTCODESIZE` could diverge. Phase 3 already handles this for steady-state; Task X.10 must spot-check `EXTCODESIZE` against a known system contract address post-genesis.

6. **The state-root field in the genesis block header diverges from canonical**. A node configured with `--binary-from-genesis` against a mainnet `genesis.json` will compute a different `state_root` and therefore a different genesis block hash. If the operator misunderstands the flag and points it at mainnet bootnodes, every peer handshake will fail with a genesis-mismatch error. **Mitigation**: log a prominent warning at startup: `warn!("Genesis-binary mode produces non-canonical block hashes; this node CANNOT follow public mainnet. Use --binary-transition for mainnet follower mode.");` Add this in `init_l1` immediately after the backend-kind selection. (Out of scope for this phase: actually preventing the misconfig — relies on operator discipline.)

7. **`peek_backend_format_byte` returns `None` for in-memory datadirs.** Task X.7's logic must treat `None` as "fresh DB" (allow). Test X.13 must include the in-memory case explicitly so the dispatch is not regressed.

8. **`compute_genesis_root` signature change cascade.** Changing the return type from `H256` to `Result<H256, StateError>` breaks 4 existing call sites in `cmd/ethrex/`. Each must be updated; if any are missed, the build fails — this is a fail-loud change, not a silent regression.

## 9. Handoff Criteria

Phase is **complete** when:

1. All tasks X.0 through X.15 + Final Audit are checked off.
2. All tests in §7 pass on both InMemory and (where applicable) RocksDB.
3. `cargo check --workspace` is clean.
4. `cargo clippy -p ethrex-storage -p ethrex-blockchain --lib --no-deps -- -D warnings` is clean.
5. `make check-cargo-lock` is clean.
6. CLI docs (`docs/CLI.md`, `docs/l1/running/startup.md`, `docs/binary-trie/operational.md`) updated for `--binary-from-genesis` (Task X.14).
7. No `TODO`/`FIXME`/`todo!()`/`unimplemented!()` introduced in this phase's diff (§2a rule 2 of master plan).
8. `docs/binary-trie/phases/phase-X-handoff.md` written, with deviations honestly documented (per master plan §10 Handoff Protocol). "Deviations: None" is allowed only if true.
9. The "Resolved Audit" section at the bottom of this file is filled in.
10. Existing transition flow tests (Phase 6, Phase 7) all still pass — additive-only verification.

## 10. Out of Scope (with destinations)

| Item | Lives in |
|---|---|
| Snap sync into binary trie | Permanent non-goal; `overview.md` lines 38–45 |
| `eth_getBinaryProof` RPC | (Renumbered) Phase 9 / current Phase 8, plan.md §6 Phase 8 |
| Metrics for binary trie | (Renumbered) Phase 9 / current Phase 8, plan.md §6 Phase 8 |
| zkVM guest integration | Permanent non-goal; `design-decisions.md` §12 |
| L2 genesis-binary support | Permanent non-goal for this PR series |
| Mainnet block-hash compatibility | Permanent non-goal; this phase explicitly produces non-canonical hashes |
| Dual-node diff harness | User explicitly rejected |
| Updates to `design-decisions.md` §11 / `overview.md` line 88 / `plan.md` §3 row | (Renumbered) Phase 10 / current Phase 9 docs phase |
| Operator runbook for genesis-binary mode | (Renumbered) Phase 10 / current Phase 9, `operational.md` updates |
| Performance benchmarking of genesis-alloc commit | (Renumbered) Phase 11 / current Phase 10 polish, Task 10.6 expansion |
| Migration tool (existing MPT DB → binary genesis) | Permanent non-goal |

## 11. Cross-References

- **Reverses**: `docs/binary-trie/design-decisions.md` §11 ("No genesis-from-binary" → genesis-from-binary IS supported behind `--binary-from-genesis`).
- **Reuses (no change)**: `design-decisions.md` §1 (overlay semantics — irrelevant here, no overlay), §2 (atomic CoW — irrelevant, no MPT base), §3 (restart-required — irrelevant, no activation), §4 (BLAKE3), §5 (sparse stems), §6 (separate FKV table), §7 (tombstones — degenerate case at genesis, no tombstones written), §8 (dual-write code), §9 (no state-root verification), §10 (one-way — trivially satisfied since no transition), §12 (witness disabled), §13 (level-parallel merkelize).
- **Inherits gates from Phase 7**: `BackendKind != Mpt` skips state-root validation and witness generation. If those gates are not yet in `blockchain.rs` at Phase 7 close, Task X.1 of this phase adds them.
- **Inherits storage primitives from Phase 5**: `BinaryBackend`, `BinaryMerkleizer`, `BinaryTrieLayerCache`, `BINARY_TRIE_NODES`, `BINARY_FLATKEYVALUE`, dual-write code path. Untouched.
- **Inherits restart logic from Phase 5/6**: `Store::from_backend` for format byte `1`. Verified, not modified.
- **Updates `plan.md` §3 row** "Genesis-binary: Unsupported" → "Genesis-binary: Supported behind `--binary-from-genesis`; mutually exclusive with `--binary-transition`; full-sync only." Edit lands in (renumbered) Phase 10 docs phase.

## 12. Open Questions

All resolved by user 2026-05-04:

1. ✓ Phase numbering: **Phase 8** (renumber existing 8/9/10 → 9/10/11).
2. ✓ Genesis state-root field: **canonical MPT genesis root** in block 0 header (Option A); binary root computed internally only.
3. ✓ Mutex enforcement: clap `conflicts_with` + manual `init_l1` check for `--syncmode=snap`.
4. ✓ Pre-existing-DB: error on format byte 0 (Mpt) or 2 (Transition); restart on byte 1 (Binary).
5. ✓ EF state-tests **not run** against the binary backend at all (user excluded). Cross-validation = LEVM unit tests + per-block always-on header verification (receipts root, transactions root, withdrawals root, gas, logs bloom).
6. ✓ `Genesis::alloc` iteration: **sorted by address** before iterating (deterministic node_id allocation).

---

## Resolved Audit

*(To be filled in by Task X.0 and X.15. Empty until phase implementation begins.)*

- [ ] State-root validation gating in `blockchain.rs` — present? location? (Task X.0)
- [ ] `Genesis::alloc` iteration determinism — `BTreeMap`, `HashMap`, or sorted Vec? (Task X.15)
- [ ] Final byte counts for the new module + diff summary. (Task X.15)
