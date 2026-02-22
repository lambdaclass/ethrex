# Phase 1.2: Sync & Hive + CI Infrastructure

**Status**: IN PROGRESS
**Branch**: `feat/tokamak-proven-execution`
**Predecessor**: Phase 1.1 (Volkov R10: 8.25 PROCEED)

---

## 1. Sync Architecture Summary

The ethrex sync subsystem lives in `crates/networking/p2p/` (~3,250 lines total).

### Core Components

| File | Lines | Role |
|------|-------|------|
| `sync_manager.rs` | 184 | Outer wrapper — holds `Syncer` behind `Arc<Mutex<>>` |
| `sync.rs` | 290 | `Syncer` struct + `SyncMode` enum + dispatch logic |
| `sync/snap_sync.rs` | 1,147 | Snap sync 9-phase algorithm |
| `sync/full.rs` | 297 | Full sync — backward header walk + batch execution |
| `sync/code_collector.rs` | 100 | Bytecode hash dedup + collection |
| `sync/healing/state.rs` | 463 | State trie healing |
| `sync/healing/storage.rs` | 740 | Storage trie healing |

### SyncMode

```rust
pub enum SyncMode {
    #[default]
    Full,
    Snap,
}
```

**Auto-switch**: `SyncManager::new()` checks if the node has prior synced state. If yes, switches from Snap to Full mode automatically.

### Snap Sync Phases

1. **Header Download** — Downloads block headers from current head to sync head via eth p2p. Falls back to full sync if too few blocks.
2. **Account Range Download** — Fetches all account trie leaves via snap protocol, writes snapshots to disk.
3. **Insert Account Ranges** — Reads leaf files, inserts into trie, computes state root.
4. **State Trie Healing + Storage Range Download** — Interleaved loop: heals state trie, fetches storage leaves. Updates pivot if stale. Falls back after 5 failed attempts.
5. **Insert Storage Ranges** — Reads storage leaf files, inserts into storage tries.
6. **Healing Process** — Iterates `heal_state_trie()` + `heal_storage_trie()` until both fully healed.
7. **Flat Key-Value Generation** — `store.generate_flatkeyvalue()`.
8. **Bytecode Download** — Deduplicates code hashes, downloads in chunks, stores via `write_account_code_batch()`.
9. **Block Body Fetch + Finalization** — Fetches pivot block body, stores it, runs `forkchoice_update()`.

### Full Sync

- Downloads headers backwards to canonical ancestor
- Executes blocks in 1024-block batches
- Triggered when node already has synced state or when snap sync falls back

---

## 2. Hive Test Matrix

### PR CI (6 Hive suites + 2 Assertoor)

Source: `.github/workflows/pr-main_l1.yaml`

| Suite | Simulation | Filter |
|-------|-----------|--------|
| RPC Compat | `ethereum/rpc-compat` | Pinned commit |
| Devp2p | `devp2p` | `discv4\|eth\|snap` |
| Engine Auth | `ethereum/engine` | `engine-(auth\|exchange-capabilities)/` |
| Engine Cancun | `ethereum/engine` | `engine-cancun` |
| Engine Paris | `ethereum/engine` | `engine-api` |
| Engine Withdrawals | `ethereum/engine` | `engine-withdrawals` |

| Assertoor | Config |
|-----------|--------|
| Transaction Check | `network_params_tx.yaml` (ethrex + geth + Lighthouse) |
| Blob & Stability | `network_params_blob.yaml` (ethrex + 2x geth + Lighthouse) |

All Hive runs: `--sim.parallelism 4 --sim.loglevel 3`.

### Daily (11 suites)

Source: `.github/workflows/daily_hive_report.yaml` (weekdays 03:00 UTC)

Above 6 suites **plus**:
- Sync tests (`ethereum/sync`)
- Consume Engine tests x3 (Paris/Shanghai/Cancun, Prague, Amsterdam)
- Consume RLP tests x3 (same fork split)
- Execute Blobs tests

Results posted to Slack.

### Snapsync (every 6h)

Source: `.github/workflows/daily_snapsync.yaml`

| Network | Timeout | CL Clients |
|---------|---------|------------|
| Hoodi | 1h | Lighthouse (`v8.0.1`), Prysm (`v7.1.0`) |
| Sepolia | 3h30m | Lighthouse (`v8.0.1`), Prysm (`v7.1.0`) |

Runs on self-hosted `ethrex-sync` runner. Build profile: `release-with-debug-assertions`.

---

## 3. Fork-Specific Changes

### 3-1. Feature Flag Split

Split monolithic `tokamak` feature into 3 independent features:

| Feature | Purpose | Propagation Path |
|---------|---------|-----------------|
| `tokamak-jit` | JIT compilation tier | `cmd/ethrex → ethrex-vm → ethrex-levm` |
| `tokamak-debugger` | Time-travel debugger | `cmd/ethrex → ethrex-vm → ethrex-levm` |
| `tokamak-l2` | Tokamak L2 hooks | `cmd/ethrex → ethrex-vm → ethrex-levm` |
| `tokamak` | Umbrella (all 3) | Enables all sub-features |

Files modified:
- `crates/vm/levm/Cargo.toml` — Defines the 3 leaf features + umbrella
- `crates/vm/Cargo.toml` — Propagates through `ethrex-levm/`
- `cmd/ethrex/Cargo.toml` — Propagates through `ethrex-vm/`

### 3-2. CI Workflow — `pr-tokamak.yaml`

New workflow triggered on PR changes to Tokamak-specific paths.

**Jobs**:
1. **quality-gate**: Checks all 4 feature combos, runs Tokamak crate tests, Clippy with `--features tokamak`
2. **format-check**: `cargo fmt --all -- --check`

### 3-3. Snapsync Image Registry

Updated `.github/actions/snapsync-run/action.yml`:
- `ethrex_image` default: `ghcr.io/lambdaclass/ethrex` → `ghcr.io/tokamak-network/ethrex`

### 3-4. Fork-Safe Components (No Changes Needed)

| Component | File | Why Safe |
|-----------|------|----------|
| Docker build action | `.github/actions/build-docker/action.yml` | Uses `${{ github.repository }}` |
| Hive client config | `.github/config/hive/clients.yaml` | Local image ref `ethrex:ci` |
| Assertoor configs | `.github/config/assertoor/*.yaml` | Local image ref `ethrex:ci` |
| Dockerfile | `Dockerfile` | No org-specific references |

---

## 4. Success Criteria

| # | Criterion | Status |
|---|----------|--------|
| 1 | `cargo check --features tokamak` (umbrella) | **PASS** |
| 2 | `cargo check --features tokamak-jit` (individual) | **PASS** |
| 3 | `cargo check --features tokamak-debugger` (individual) | **PASS** |
| 4 | `cargo check --features tokamak-l2` (individual) | **PASS** |
| 5 | `cargo test --workspace` passes (718 tests, 0 failures) | **PASS** |
| 6 | `pr-tokamak.yaml` triggers and passes on PR | PENDING (CI) |
| 7 | Docker build succeeds on fork | PENDING (CI) |
| 8 | Hive PR suites pass (baseline recorded) | PENDING (CI) |
| 9 | Snapsync completes on Hoodi | PENDING (CI) |

---

## 5. Next Steps

- **Phase 1.3**: Benchmarking Foundation — `tokamak-bench` implementation, `perf_opcode_timings` CI integration
