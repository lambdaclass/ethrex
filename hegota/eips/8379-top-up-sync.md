# EIP-8379: Top-up Sync
- **Layer:** CL (Engine API / EL sync behavior)
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI, acdc/185, 2026-08-20)
- **Authors:** Jacek Sieka, Dustin, Tamaghna Choudhuri
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8379) · [discussion](https://ethereum-magicians.org/t/eip-8379-top-up-sync/29405)
- **Prior fork history:** none

## TL;DR
Adds `engine_getSyncStatus` (reporting the EL's block head and executed head separately) and a new `MISSING_STATE` payload status that distinguishes "missing ancestor blocks" from "missing pre-state". This lets the CL deterministically drive EL block sync by feeding each block immediately after the EL head via `engine_newPayload` — a path toward deprecating devp2p block-distribution messages.

## What it changes
- **New engine method `engine_getSyncStatus`** (500ms timeout, advertised via `engine_exchangeCapabilities`) returning:
  - `headBlockHash`/`headBlockNumber`: head of the contiguous canonical chain of *held* blocks rooted in the executed head (or genesis).
  - `executedBlockHash`/`executedBlockNumber` (nullable): latest canonical block whose post-state the EL holds — the block whose child the EL could execute.
  - Guarantees: a `newPayload` whose parent is the reported block head MUST NOT be answered `SYNCING`; the report only changes after a processed `newPayload`/`forkchoiceUpdated`.
- **New payload status `MISSING_STATE`** on the new `engine_newPayload`/`engine_forkchoiceUpdated` versions: returned when all ancestor blocks are known but the pre-state is unavailable. Payloads answered `MISSING_STATE` MUST be persisted, extending the known-block chain without executing. `SYNCING` is then reserved strictly for missing ancestor block data.
- **EL freedom to close the gap** between executed head and block head by any means: replay stored payloads, snap sync, or EIP-7928 BAL-based fast-forward.
- **Top-up procedure (CL side):** loop `getSyncStatus` → deliver the canonical block at `headBlockNumber + 1` → handle forks by locating the fork point → if the needed block is older than retained history, the EL must state-sync by other means.
- **Cross-fork payloads:** the new `newPayload` version MUST accept payloads from *earlier* forks (applying that fork's rules) so a top-up can span fork boundaries — a deliberate break from one-version-per-fork versioning.
- CL behavior: `SYNCING` → supply ancestors in ascending order; `MISSING_STATE` → keep delivering forward, never treat it as validated.

## Motivation
Today ELs duplicate block distribution on devp2p while CLs already have the canonical chain via gossip/req-resp. The CL cannot drive EL sync because (a) it cannot query the EL's head(s), and (b) a `SYNCING` response is ambiguous — missing ancestors (CL should send them) vs. missing state (sending ancestors is useless). Disambiguating both makes the newPayload/FCU choreography deterministic and opens the door to deprecating `GetBlockHeaders`/`GetBlockBodies` on devp2p (snap sync and mempool stay).

## Dependencies & related EIPs
- Synergizes with **EIP-7928** (BALs, Glamsterdam): named as a fast-forward mechanism to advance the executed head without full re-execution.
- Complements **EIP-8237** (independent CL/EL sync): 8237 defers payload verification during CL range sync; 8379 handles delivering blocks from CL to EL afterward.
- Related to **EIP-8383** (CL block retention): the retention window bounds how far back top-up sync can reach.
- Mentions the REST/SSZ engine API transport mapping (non-normative).
- Long-term: enables deprecating devp2p eth-protocol block messages (relevant to networking EIPs like **EIP-8133**-class mempool/devp2p work).

## Impact on ethrex / client teams
Direct Engine API work in `crates/networking/rpc/engine`: implement `engine_getSyncStatus`, extend `PayloadStatus` with `MISSING_STATE`, persist-but-don't-execute such payloads, and accept historical-fork payloads in the new method version. Semantics interact with ethrex's sync code (`crates/networking/p2p/sync_manager.rs`, `sync/full.rs`) and fork choice (`crates/blockchain/fork_choice.rs`) — the "two heads" model must map onto ethrex's notion of canonical head vs. executed state. **Effort: medium.** No EVM, gas, or consensus-rule changes.

## Open questions & controversies
- Unbounded buffering: payloads answered `MISSING_STATE` must be persisted; pruning policy for them is explicitly undecided.
- Multiple CLs multiplexed on one EL void the `getSyncStatus` stability guarantee; normative requirements on multiplexer software are a TODO.
- Deprecating devp2p block download is a big philosophical shift some EL teams may resist (loses an independent block source).
- Batched payload delivery and reduced verification for finalized blocks deliberately deferred to future EIPs.
