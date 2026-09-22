# EIP-8142: Block-in-Blobs (BiB)
- **Layer:** CL (with significant EL/Engine API changes)
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI, acdc/184, 2026-08-06)
- **Authors:** Kevaundray Wedderburn, Ignacio Hagopian, Jihoon Song, Francesco Risitano, Thomas Thiery, Toni Wahrstätter, Péter Garamvölgyi
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8142) · [discussion](https://ethereum-magicians.org/t/eip-8142-block-in-blobs-bib/27621)
- **Prior fork history:** none

## TL;DR
Once zkEVM proofs let validators verify blocks without downloading them, nothing forces a builder to publish the block's contents — they could keep the data and monopolize building the next block. BiB fixes this by requiring the execution-payload data (transactions + block access list) to be encoded into blobs carried by the same beacon block, so existing DAS machinery guarantees the data is available.

## What it changes
- **New EL header field `payload_blob_count: uint64`:** the first `payload_blob_count` KZG commitments referenced by the beacon block commit to payload-blobs; the rest belong to type-3 (EIP-4844) blob transactions.
- **Payload data encoding:** `ExecutionPayloadData = blockAccessList (RLP, per EIP-7928) + transactions (RLP list)`, prefixed with an 8-byte header `[4B BAL length][4B tx length]` (big-endian) so the BAL can be extracted without parsing txs. Packed into blobs using 31 usable bytes per field element (MSB zeroed for canonical BLS field elements), zero-padded. No compression for now (adding it later would be breaking).
- **`engine_getPayload`:** the builder computes payload blobs, sets `payload_blob_count`, and returns payload blobs **first**, then type-3 blobs, in the `BlobsBundle`. Two variants: native (uses `blob_to_kzg_commitment`) and zkEVM-optimized (adds `payload_kzg_proofs` — random-point opening proofs as private circuit inputs, avoiding MSM in-circuit).
- **`engine_newPayload`:** before running the STF, the EL re-derives the payload blobs from the payload, asserts the count matches the header, and asserts `expected_blob_versioned_hashes == payload hashes ++ type-3 hashes`. zk variant verifies blob–commitment consistency via `verify_blob_kzg_proof_batch` instead.
- **Fee accounting:** payload blobs consume blob gas and compete with transaction blobs under `MAX_BLOBS_PER_BLOCK`, but pay no per-blob fee — treated as a protocol-accepted cost.
- **Networking:** reuses blob gossip/DAS; notes a future fallback where nodes reconstruct the payload from blobs and re-seed the `execution_payload` topic if a builder gossips only the header (relevant once proofs are mandatory).

## Motivation
Re-execution today implicitly guarantees DA: you can't verify a block you didn't download. zkEVM validation removes that guarantee. A withholding builder would be the only party able to build on the chain, and RPC/indexer nodes (which must re-execute) would stall. Publishing the payload data as blobs restores the DA invariant through PeerDAS.

## Dependencies & related EIPs
- **Requires EIP-4844** (blobs), **EIP-7594** (PeerDAS, Fusaka), **EIP-7732** (ePBS, Glamsterdam), **EIP-7892** (BPO forks — `MAX_BLOBS_PER_BLOCK` alignment), **EIP-7928** (BALs, Glamsterdam — the BAL is half the published data).
- Conflicts/interacts with **EIP-8146** (BAL sidecars): 8146 removes the BAL from the payload; 8142 puts it into blobs. If both land, the encoding/ordering story needs reconciliation.
- Notes **EIP-7807** (SSZ execution blocks) as a future switch from RLP to SSZ for the encoding.
- Synergizes with **EIP-8371** (RowDAS): more blobs per block from BiB increases reconstruction load that RowDAS distributes.

## Impact on ethrex / client teams
Substantial EL work: header schema change (`payload_blob_count`) in `crates/common/types`, blob packing/unpacking (31-byte field-element chunks), KZG commitment/proof computation at the engine boundary (`crates/networking/rpc/engine`), changes to both payload building (`crates/blockchain/payload.rs`, `engine_getPayload`) and validation (`engine_newPayload`), plus blob-gas accounting interplay with type-3 txs. ethrex already has BAL and KZG plumbing from EIP-7928/4844 work, which helps. The zkEVM-optimized variant aligns with ethrex's own proving stack (guest-program). **Effort: medium-large**, and only meaningful on the ePBS + BAL + PeerDAS stack.

## Open questions & controversies
- Payload blobs eat into the same blob capacity as L2 blobs with no fee — blob base fee dynamics and explicit protocol pricing are listed open questions.
- Compression deferred: current design is uncompressed; switching later is a breaking change.
- Test cases and reference implementation are TODO — spec immaturity.
- Depends on the whole not-yet-shipped Gloas stack (ePBS + BAL), so it's a forward-looking design for the mandatory-proofs era; some may see it as premature.
- Builder bandwidth: payload blobs were never in the mempool, so builders carry extra propagation load.
