# EIP-8116: Replace cumulative receipt fields
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) — acde/242, 2026-07-30 (champion: Etan Kissling)
- **Authors:** Etan Kissling, Gajinder Singh
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8116) · [discussion](https://ethereum-magicians.org/t/eip-8116-replace-cumulative-receipt-fields/27359)
- **Prior fork history:** none

## TL;DR
Change the on-chain receipt so it records each transaction's own `gasUsed` instead of the running block total (`cumulativeGasUsed`), and change the RPC `logIndex` field to be per-receipt instead of per-block. Receipts become position-independent, so verifying one transaction's gas no longer requires the preceding receipt and receipts no longer serialize parallel execution.

## What it changes
- **Receipt construction (consensus):** all receipts emitted after activation store the individual transaction's `gasUsed` in place of `cumulativeGasUsed`. This changes the receipts trie root / `receipts_root` computation.
- **JSON-RPC:** within `logs`, `logIndex` now counts logs within the individual receipt, not within the whole block. Block-wide ordering can be emulated as `transactionIndex * 4 + logIndex` (same relative order, absolute values may differ).
- `cumulativeGasUsed` for a position can still be derived by summing, so RPC responses could keep exposing it computed — but the on-chain data changes meaning.

## Motivation
1. **Verification efficiency:** an RPC client verifying a single transaction's gas currently needs two consecutive receipts to subtract `cumulativeGasUsed` values.
2. **Parallelism:** receipt contents are stateful (depend on all prior transactions) even when transactions touch disjoint state — an artificial serialization point for execution and receipt proving.
3. **Alignment:** on-chain receipts should match the per-transaction data applications actually consume over RPC.

## Dependencies & related EIPs
- **Depends on:** nothing — standalone receipt-format change.
- **Synergizes with:** EIP-8115 (same authors, same theme of removing per-tx position-dependent state), EIP-7807 (SSZ execution blocks redefines `receipts_root` as an SSZ root; position-independent receipts complement SSZ proving), EIP-7928 (BAL / parallel execution — receipt statefulness is one of the blockers).
- **Conflicts / overlap with:** anything hard-coding the current receipt layout; interacts with any fork's redefinition of the receipts trie (e.g. 7807's SSZ receipts list).

## Impact on ethrex / client teams
- Touches: `Receipt` type (`crates/common/types/receipt.rs` — `cumulative_gas_used` field semantics), receipts trie computation in block processing, wire receipt encodings (`crates/networking/p2p/rlpx/eth/eth68|eth70/receipts.rs`), RPC receipt types (`crates/networking/rpc/types/receipt.rs` — `logIndex` computation), and RPC block/log serving.
- DB: stored receipts could keep either form (clients may store computed values), but trie roots must follow the new rule from the fork block.
- Effort: **small-to-medium** — mechanically simple, but the field-meaning change ripples through RPC, indexers, and tests; eth_getLogs / receipt RPC consumers need a fork-aware switch.

## Open questions & controversies
- Breaking change for applications that read `gasUsed`/`cumulativeGasUsed` semantics or block-level `logIndex` from receipts/RPC; migration burden sits on indexers, explorers, and L2s that consume receipts.
- `logIndex` becomes ambiguous across the fork boundary — indexers must handle both meanings.
- Otherwise uncontroversial; framed by the authors as one step in aligning on-chain data with RPC consumption.
