# ethrex Architecture Overview

*Analyzed: 2026-02-22 | Base commit: `36f9bf7a8` on `feat/tokamak-proven-execution`*

## Project Scale

- **Workspace members**: 25 crates + 2 non-member path dependencies (`ethrex-metrics`, `ethrex-monitor`)
- **Default member**: `cmd/ethrex` only (other crates compile on demand)
- **Codebase**: ~133K lines Rust (excluding `target/`)
- **Edition**: Rust 2024, resolver v2
- **License**: MIT OR Apache-2.0 (workspace-wide)

## Crate Dependency Graph

```
Layer 0 (Leaf — no internal deps):
  ethrex-rlp    ethrex-crypto    ethrex-sdk-contract-utils    ethrex-repl

Layer 1:
  ethrex-trie ──> ethrex-crypto, ethrex-rlp

Layer 2:
  ethrex-common ──> ethrex-rlp, ethrex-trie, ethrex-crypto

Layer 3:
  ethrex-storage ──> ethrex-common, ethrex-crypto, ethrex-rlp, ethrex-trie
  ethrex-levm   ──> ethrex-common, ethrex-crypto, ethrex-rlp
  ethrex-metrics ──> ethrex-common

Layer 4:
  ethrex-vm      ──> ethrex-common, ethrex-crypto, ethrex-levm, ethrex-trie, ethrex-rlp
  ethrex-l2-common ──> ethrex-common, ethrex-crypto, ethrex-rlp, ethrex-trie, ethrex-vm

Layer 5:
  ethrex-blockchain ──> ethrex-common, ethrex-crypto, ethrex-storage,
                        ethrex-trie, ethrex-vm, ethrex-metrics, ethrex-rlp
  ethrex-storage-rollup ──> ethrex-common, ethrex-storage, ethrex-trie,
                            ethrex-rlp, ethrex-l2-common
  ethrex-guest-program ──> ethrex-common, ethrex-crypto, ethrex-vm,
                           ethrex-rlp, ethrex-l2-common

Layer 6:
  ethrex-p2p ──> ethrex-common, ethrex-crypto, ethrex-blockchain,
                 ethrex-rlp, ethrex-storage, ethrex-trie
                 [optional: ethrex-storage-rollup, ethrex-l2-common, ethrex-metrics]

Layer 7:
  ethrex-rpc  ──> ethrex-common, ethrex-storage, ethrex-vm, ethrex-blockchain,
                  ethrex-metrics, ethrex-crypto, ethrex-p2p, ethrex-rlp, ethrex-trie
  ethrex-config ──> ethrex-p2p, ethrex-common

Layer 8:
  ethrex-dev    ──> ethrex-rpc
  ethrex-l2-rpc ──> ethrex-common, ethrex-storage, ethrex-blockchain, ethrex-p2p,
                    ethrex-storage-rollup, ethrex-l2-common, ethrex-rpc, ethrex-rlp

Layer 9:
  ethrex-sdk    ──> ethrex-common, ethrex-rpc, ethrex-l2-common, ethrex-l2-rpc,
                    ethrex-sdk-contract-utils, ethrex-rlp
  ethrex-monitor ──> ethrex-common, ethrex-config, ethrex-l2-common, ethrex-sdk,
                     ethrex-rlp, ethrex-rpc, ethrex-storage, ethrex-storage-rollup

Layer 10:
  ethrex-l2 ──> 18 internal deps (highest fan-out crate)

Layer 11:
  ethrex-prover ──> ethrex-common, ethrex-storage, ethrex-vm, ethrex-rlp,
                    ethrex-blockchain, ethrex-l2, ethrex-l2-common, ethrex-sdk,
                    ethrex-guest-program

Layer 12 (Binary):
  ethrex (cmd) ──> ethrex-blockchain, ethrex-common, ethrex-config, ethrex-crypto,
                   ethrex-metrics, ethrex-p2p, ethrex-repl, ethrex-rlp, ethrex-rpc,
                   ethrex-storage, ethrex-vm
                   [optional L2: ethrex-dev, ethrex-l2, ethrex-l2-common, ethrex-l2-rpc,
                    ethrex-prover, ethrex-sdk, ethrex-storage-rollup]
```

**Key observations:**

- `ethrex-common` is the most depended-upon crate (nearly universal dependency)
- `ethrex-l2` has the highest fan-out at 18 internal dependencies
- L2 functionality is entirely optional, gated behind the `l2` feature flag
- Prover backends (`sp1`, `risc0`, `zisk`, `openvm`) propagate cleanly from binary through the stack

## Node Startup Flow

```
main() [ethrex.rs:142]
  ├── CLI::parse()                      — clap-based argument parsing
  ├── rayon::ThreadPoolBuilder           — global thread pool for parallel work
  ├── init_tracing()                     — tracing + EnvFilter + optional file logging
  └── init_l1() [initializers.rs:430]
        ├── get_network()                — Mainnet / Holesky / Sepolia / Hoodi / Custom Genesis
        ├── init_store()                 — Storage backend (RocksDB required at compile time)
        ├── init_blockchain()            — Blockchain (mempool, perf logging, witness precompute)
        ├── regenerate_head_state()      — Rebuild state from latest block if needed
        ├── get_signer() + P2P node      — secp256k1 signing key for P2P identity
        ├── PeerTable::spawn()           — Peer discovery and management
        ├── P2PContext::new()            — RLPx initiator + listener
        ├── SyncManager::spawn()         — Snap/Full sync orchestration
        ├── RPC::start()                 — JSON-RPC (HTTP + Engine API + Auth)
        ├── Metrics::start()             — Prometheus metrics endpoint (optional)
        └── REPL::start()               — Interactive CLI (optional)
```

## Supported Networks

| Network | Source |
|---------|--------|
| Mainnet | Built-in genesis + chainspec |
| Holesky | Built-in genesis + chainspec |
| Sepolia | Built-in genesis + chainspec |
| Hoodi   | Built-in genesis + chainspec |
| Custom  | `--network` flag with genesis JSON path |

## Build Profiles

| Profile | Settings |
|---------|----------|
| **dev** | `debug = 2` |
| **release** | `opt-level = 3`, `lto = "thin"`, `codegen-units = 1` |
| **release-with-debug** | inherits release + `debug = 2` |
| **release-with-debug-assertions** | inherits release + `debug-assertions = true` |

## Feature Flags

### Binary-level (`cmd/ethrex`)

| Feature | Effect |
|---------|--------|
| **default** | `rocksdb`, `c-kzg`, `secp256k1`, `metrics`, `jemalloc`, `dev` |
| `l2` | Enable L2 sequencer/operator (ethrex-l2, prover, rollup storage) |
| `sp1` / `risc0` | ZK prover backends |
| `perf_opcode_timings` | Forward to ethrex-vm for opcode-level profiling |
| `jemalloc` | tikv-jemallocator global allocator |
| `jemalloc_profiling` | jemalloc + heap profiling |
| `cpu_profiling` | pprof-based CPU profiling |
| `sync-test` | Forward to ethrex-p2p for sync testing |
| `experimental-discv5` | Discovery V5 protocol (experimental) |

### EVM-level (`ethrex-levm`)

| Feature | Effect |
|---------|--------|
| **default** | `secp256k1` |
| `c-kzg` | KZG commitment support |
| `ethereum_foundation_tests` | EF test suite integration |
| `debug` | Debug mode |
| `sp1` / `risc0` / `zisk` / `openvm` | ZK VM backend compilation |
| `perf_opcode_timings` | Per-opcode timing instrumentation |

## CI Workflows

ethrex ships with 29 GitHub Actions workflows. Key ones for Tokamak:

### Must Keep (L1/LEVM core)

| Workflow | Purpose |
|----------|---------|
| `pr-main_l1.yaml` | L1 lint + test + Hive integration tests |
| `pr-main_levm.yaml` | LEVM + Ethereum Foundation tests |
| `pr_perf_levm.yaml` | LEVM performance benchmarks |
| `pr-main_l1_ef_tests.yaml` | Ethereum Foundation test suite |

### Keep (Infrastructure)

| Workflow | Purpose |
|----------|---------|
| `pr_lint_gha.yaml` | GitHub Actions linting |
| `pr_lint_license.yaml` | License compliance check |
| `pr_lint_pr_title.yml` | PR title format validation |
| `pr_loc.yaml` | Lines of code analysis |
| `pr_perf_changelog.yml` | Performance changelog enforcement |

### Skip Until Needed

| Workflow | Purpose | When Needed |
|----------|---------|-------------|
| `pr-main_l2.yaml` | L2 lint + test | Phase 4 (Tokamak L2) |
| `pr-main_l2_prover.yaml` | L2 prover tests | Phase 4 |
| `pr_upgradeability.yaml` | L2 contract upgradeability | Phase 4 |
| `assertoor_*.yaml` (4 workflows) | Multi-client testing | Phase 5 |
