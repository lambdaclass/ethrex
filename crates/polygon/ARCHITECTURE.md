# Polygon PoS — Architecture

ethrex implements Polygon PoS as a chain variant alongside Ethereum L1 and L2 rollups. This document explains how it works and where it diverges from base ethrex.

## Overview

On L1, ethrex is a passive executor — a Beacon Chain consensus client pushes the canonical head via the Engine API, and ethrex validates and stores. On Polygon there is no Beacon Chain — ethrex drives its own consensus. It verifies block producers against an internally-tracked validator set, queries Heimdall for consensus data, and runs fork choice.

| Area | L1 | Polygon |
|---|---|---|
| Sync trigger | Engine API `forkchoiceUpdated` | P2P polling bridge (500ms) |
| Block validation | Stateless (check header fields) | Stateful (validator snapshots evolve per block) |
| Fork choice | Beacon Chain decides | Total difficulty comparison |
| Block finalization | Compute state root | System calls + code upgrades, then state root |

## Background

Polygon PoS has a two-component architecture: **Heimdall** handles validator selection, state sync, and finality checkpoints; **Bor** is the block-producing execution layer. "Bor" is both Polygon's reference implementation (a geth fork) and the name of this architectural role. ethrex fills the Bor role, which is why names like `BorEngine`, `BorConfig`, and the `bor/` RPC namespace appear throughout the codebase.

Polygon has its own hard fork schedule, activated by block number rather than timestamp.

**Spans** (~6400 blocks) define which validators are eligible to produce blocks. Heimdall selects each span's validator set based on staking weight, and validators are read directly from Heimdall.

**Sprints** (16 blocks) are sub-divisions of a span. Two things happen at sprint boundaries:
- At **sprint starts** (block 0, 16, 32, ...), state sync events from L1 are committed on-chain via `commitState`.
- At **sprint ends** (block 15, 31, 47, ...), the block proposer rotates.

**Heimdall** runs as a separate process. ethrex queries it over a REST API for spans and state sync events.

## Architecture

```
┌──────────────┐                     ┌──────────────────────────────────────────────────┐
│              │  REST API           │                    ethrex                        │
│   Heimdall   │◄───────────────────►│                                                  │
│              │  spans,              │  ┌──────────────────────────────────────┐        │
│              │  state sync events  │  │            BorEngine                 │        │
│              │                     │  │                                      │        │
│              │                     │  │  SnapshotCache ──► verify_header()   │        │
└──────────────┘                     │  │                                      │        │
                                     │  │                                      │        │
       ┌─────────────────────────────│──│─ HeimdallPoller                      │        │
       │  background polling:        │  │  (updates span)                      │        │
       │  every 1s / 5s / 2s         │  │                                      │        │
       └─────────────────────────────│──│─ current_span ──► need_to_commit_    │        │
                                     │  │                    span()            │        │
                                     │  │  milestone       (not yet wired up)  │        │
                                     │  └──────────┬───────────────────────────┘        │
                                     │             │                                    │
                                     │             ▼                                    │
                                     │  ┌──────────────────────┐  ┌──────────────────┐  │
                                     │  │     Blockchain       │  │   LEVM           │  │
                                     │  │                      │  │                  │  │
┌──────────────┐                     │  │  verify_bor_header() │  │  PolygonHook     │  │
│  Bor Peers   │◄───── P2P ─────────►│  │  execute_block()     │  │  (fee split,     │  │
│              │  set_polygon_sync_  │  │  system_calls()      │  │   synthetic logs)│  │
│              │  head() on status   │  │  block_alloc()       │  │                  │  │
│              │  exchange           │  └──────────────────────┘  │  Polygon opcode  │  │
└──────────────┘                     │             ▲              │  table           │  │
                                     │             │              └──────────────────┘  │
                                     │  ┌──────────┴───────────┐                        │
                                     │  │    Sync Bridge       │                        │
                                     │  │  (500ms poll loop,   │                        │
                                     │  │   replaces Engine    │                        │
                                     │  │   API FCU trigger)   │                        │
                                     │  └──────────────────────┘                        │
                                     └──────────────────────────────────────────────────┘
```

Blocks arrive through two paths. The primary path is **NewBlock messages** from peers — when a NewBlock arrives and the parent is known, ethrex executes it immediately, then chains through any buffered children. Out-of-order blocks are buffered by parent hash until their parent arrives. The **sync bridge** is the fallback: when NewBlock messages stop flowing (e.g., during initial sync or after a gap), it triggers the sync manager to fetch blocks by number. The Engine API is disabled — `engine_*` calls return `MethodNotFound`.

## Block execution

Block execution follows the same validate → execute → merkleize → store flow as L1, with Polygon-specific steps marked below:

```
1. verify_bor_header() — structural checks, seal recovery, signer authorization,
   proposer rotation at sprint ends

2. Resolve PolygonFeeConfig — recover block author, look up burnt_contract and
   BorConfig coinbase for this block number

3. Execute transactions (PolygonHook replaces DefaultHook)

4. System calls (post-transaction, pre-state-root)
   └── commitState — at sprint starts, using StateSyncTransaction data from block body

5. Block alloc — at fork blocks, deploy/upgrade system contract code in state

6. Merkleize + store
```

## Consensus

### BorEngine

Central orchestrator (`consensus/engine.rs`). Holds the BorConfig, a Heimdall HTTP client, a snapshot cache, the latest milestone, and the current span. The mutable fields are synchronized between the background Heimdall poller and the block execution thread.

### Validator snapshots

A `Snapshot` tracks the validator set and recent signers, evolving with every block:

- **apply_header** — recovers the block signer from the seal, verifies they're in the validator set, and tracks recent signers (used for difficulty calculation in Bor).
- **increment_proposer_priority** — at sprint ends, rotates the proposer using a weighted round-robin derived from Tendermint. Higher-stake validators get more frequent turns.

### Bootstrap after snap sync

After snap sync there are no historical headers to reconstruct a snapshot. `bootstrap_snapshot()` fetches the current span from Heimdall and builds an initial snapshot for the pivot block.

### Fork choice

Total-difficulty-based: higher TD wins, then higher block number, then lower block hash. Bor uses Heimdall milestones to prevent deep reorgs; ethrex instead relies on the storage layer's 128-block commit threshold.

## Fee distribution

L1 pays all fees to `header.coinbase`. Polygon splits them per-transaction via `PolygonHook`:

- **Base fee × gas_spent** → `burnt_contract` (from BorConfig, varies by block number)
- **Tip × gas_spent** → BorConfig coinbase

## System calls

After regular transactions but before the state root, Polygon blocks may execute system calls from a special system address to on-chain contracts. These don't consume block gas.

**commitState** — at sprint starts, reads `StateSyncTransaction` (a consensus-only transaction carrying Heimdall state sync data) from the block body and calls the StateReceiver contract for each event. Reverts are non-fatal.

**Block alloc** — at fork block numbers, applies code and balance overrides directly to state (not through the EVM) to deploy or upgrade system contracts.

## EVM

Bor injects synthetic logs into transaction receipts (fee transfers, native value transfers). ethrex reproduces these via `PolygonHook` and inline log emission in CALL/CREATE handlers to match Bor's receipts root.

Opcode and precompile differences from Ethereum:

- `COINBASE` returns the BorConfig coinbase (fee/tip recipient), not the block author. `header.coinbase` is always `0x0`.
- `PREVRANDAO` returns `header.difficulty` — Polygon has real difficulty, not beacon randomness.
- `BLOBHASH` and `BLOBBASEFEE` are invalid — Polygon has no blobs.
- `CLZ` is gated on `Fork::Lisovo`.
- KZG point evaluation precompile is active only between `Fork::Lisovo` and `Fork::LisovoPro`.
- P256Verify (0x100) is always active.

## Headers

Polygon headers include `base_fee_per_gas` but lack post-merge fields (withdrawals, blobs, beacon block root, execution requests) since Polygon never went through The Merge.

Key differences from Ethereum headers:
- **coinbase** — always `0x0`; producer recovered from `extra_data` signature
- **difficulty** — non-zero (1 or 2), encoding whether the signer is the in-turn proposer.
- **extra_data** — variable length: `[32B vanity][payload][65B signature]`. The payload is RLP-encoded `BlockExtraData`.

## Mempool

The mempool rejects blob transactions and enforces a 25 Gwei minimum gas price.

## Fork configuration

Polygon forks are block-number-activated. The `Fork` enum has 12 Polygon-specific variants. Most activated millions of blocks ago; ethrex handles them for completeness and testnet compatibility. `BorConfig` stores parameters that change at fork boundaries (sprint size, block period, contract addresses, etc.) as block-number-indexed maps with floor-key lookup.

## Code map

```
crates/polygon/
├── src/
│   ├── bor_config.rs          Dynamic consensus parameters (sprint, period, forks)
│   ├── genesis.rs             Genesis construction + BorConfig lookup by chain ID
│   ├── fork_id.rs             EIP-2124 fork ID (Polygon variant)
│   ├── validation.rs          Structural header checks
│   ├── system_calls.rs        ABI encoding for commitSpan/commitState
│   ├── consensus/
│   │   ├── engine.rs          BorEngine: verify, fork choice, milestone, bootstrap
│   │   ├── snapshot.rs        Validator snapshots + proposer rotation
│   │   ├── seal.rs            Seal hash + signer recovery
│   │   └── extra_data.rs      Extra data parsing (raw + RLP formats)
│   └── heimdall/
│       ├── client.rs          REST client with retry
│       ├── types.rs           Span, Validator, EventRecord, Milestone
│       └── poller.rs          Background polling
├── allocs/                    Genesis allocations (mainnet 137, Amoy 80002)
└── tests/

Integration points:
  blockchain/blockchain.rs             BlockchainType::Polygon, system calls, block alloc
  vm/levm/src/hooks/polygon_hook.rs    Fee distribution + synthetic logs
  vm/levm/src/vm.rs                    VMType::Polygon, LogTransfer, warm set
  vm/levm/src/opcodes.rs               Polygon opcode table
  vm/levm/src/precompiles.rs           KZG/P256Verify fork gating
  common/types/transaction.rs          StateSyncTransaction (0x7F)
  common/types/genesis.rs              Fork enum with Polygon variants
  common/config/networks.rs            Polygon/Amoy networks + bootnodes
  networking/p2p/sync/full.rs          Forward-sync-by-number
  networking/p2p/rlpx/eth/*/status.rs  Polygon fork ID in P2P handshake
  networking/rpc/bor/mod.rs            bor_getAuthor, bor_getRootHash
  cmd/ethrex/initializers.rs           Sync bridge task
```
