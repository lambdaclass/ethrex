# EIP-8094: eth/vhash - Blob-Aware Mempool
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) (acde/243, 2026-08-13)
- **Authors:** Csaba Kiraly
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8094) · [discussion](https://ethereum-magicians.org/t/eip-8094-eth-vhash-blob-aware-mempool/26834)
- **Prior fork history:** none

## TL;DR
Makes mempool messaging aware of blob versioned hashes: type-3 (blob) transactions are propagated *without* their sidecars, and blob content is fetched separately by vhash via new `GetPooledBlobs`/`PooledBlobs` messages. A fee-bump (RBF) of a blob transaction no longer resends the unchanged blob data across the network.

## What it changes
- `Transactions (0x02)` and `PooledTransactions (0x0a)`: type-3 transactions are sent **without** sidecars.
- New `GetPooledBlobs` message: `[request-id, [vhash₁: B_32, …]]` requests blobs from a peer's tx pool by versioned hash.
- New `PooledBlobs` response: `[request-id, [blob₁, blob₂, …]]` — blobs in request order, but unavailable blobs may be skipped (partial responses allowed). Open TODOs on the blob format (version prefix; possibly shipping blob content + commitment + cell proofs so receivers can reconstruct PeerDAS-era RS encoding CPU-efficiently).
- Reverses the EIP-4844 broadcast rule: nodes MUST propagate blob transactions (without sidecars, push or announce) instead of announce-only; nodes MUST NOT forward a blob transaction until its blobs are fetched and validated.
- Requires a new `eth` protocol version; no consensus change, no hard fork. Requires **EIP-7642** (eth/69).

## Motivation
Under eth/69, replacing a blob transaction re-distributes the full payload even though a typical RBF changes only fees. RBF happens most during fee volatility — exactly when the network is already congested — so the current behavior multiplies load at the worst time. Exposing vhashes in mempool messaging lets nodes skip re-downloading blob content they already hold. A side benefit: since sidecar-less blob transactions are small, they can be eagerly pushed again.

## Dependencies & related EIPs
- Requires **EIP-7642** (eth/69 baseline).
- Combinable with **EIP-8077** (announce with source+nonce): 8077 announcements would let a node detect a same-nonce replacement and decide whether it already holds the sidecars; the EIPs are designed to compose.
- Overlaps with **EIP-8070** (sparse blobpool): both change blob propagation; goals differ (8070 = proportional bandwidth reduction, 8094 = cheap RBF). The combination is explicitly left unresolved ("depends on order of introduction", TODO).
- Interacts with PeerDAS (**EIP-7594**, Fusaka) blob formats (cell proofs) in the blob transfer encoding.

## Impact on ethrex / client teams
Networking + mempool. ethrex's eth protocol code is versioned per directory (`crates/networking/p2p/rlpx/eth/eth68 … eth72`); add the two new message codecs in `transactions.rs`/a new module, plumb through `rlpx/connection/server.rs`, and change blob-tx handling in the tx broadcaster and mempool (`crates/networking/p2p/tx_broadcaster.rs`, `crates/blockchain/mempool.rs`): track sidecars keyed by vhash separately from transactions, gate forwarding on blob validation, and serve/answer `GetPooledBlobs`. Effort: medium (new message types, pool bookkeeping, partial-response handling).

## Open questions & controversies
- Adds one RTT per hop to blob propagation since data is fetched separately from the transaction.
- Partial responses are allowed (peers may skip unavailable blobs), so fetchers need retry/completeness logic.
- Interaction with EIP-8070 (sparse blobpool) explicitly unresolved in the spec.
- Message codes still "to be assigned"; blob format details (version prefix, cell-proof bundling) are open TODOs — spec is immature.
