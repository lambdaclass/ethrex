# EIP-7807: SSZ execution blocks
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Declined (acde/232, 2026-03-12) then re-Proposed (CFI) (acde/242, 2026-07-30); former headliner candidate (presented acde/229, 2026-01-29), not currently a headliner
- **Authors:** Etan Kissling, Gajinder Singh
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-7807) · [discussion](https://ethereum-magicians.org/t/eip-7807-ssz-execution-blocks/21580)
- **Prior fork history:** Declined from Hegota once, re-proposed; was a headliner candidate

## TL;DR
Migrates the execution block (header/payload, block hash, per-block tries) from RLP + Merkle-Patricia to SSZ, unifying the block representation with the consensus layer. The block hash becomes the SSZ `hash_tree_root` (SHA-256) everywhere — BLOCKHASH opcode, JSON-RPC, devp2p, CL references. Individual txs/receipts/withdrawals keep their existing typed-envelope/RLP encodings; the state trie is untouched.

## What it changes
- New `ExecutionPayload` as an SSZ `ProgressiveContainer` (18 fields): PoW-era fields dropped (ommer/uncle roots, difficulty), logs bloom dropped from the block; gas fields normalized into `GasAmounts{regular, blob}` and `BlobFeesPerGas` structs; adds `block_access_list` and `slot_number`.
- `receipts_root` becomes `hash_tree_root` over a `ProgressiveList` of receipts; `requests_hash` reuses the CL `ExecutionRequests` root. Header = SSZ summary of the payload (lists replaced by their roots).
- **Block hash redefinition:** `ExecutionPayload.hash_tree_root()` used in (1) BLOCKHASH opcode, (2) RPC `blockHash`, (3) devp2p, (4) CL `BeaconState.latest_block_hash` / `ExecutionPayloadBid.block_hash`. Hash function switches from keccak256 to SHA-256.
- Engine API can drop the redundant `block_hash` field and move to binary SSZ (`ForkDigest`-context) encoding — ~50% less CL↔EL traffic.
- JSON-RPC `logsBloom` field of blocks must be returned as all-`1`s (256 bytes of `0xff`), forcing consumers to scan receipts; per-receipt blooms unaffected.
- `ProgressiveContainer`/`ProgressiveList` keep generalized indices stable, so proofs of individual fields stay forward-compatible.

## Motivation
Today EL data is RLP/MPT while CL is SSZ, forcing constant format conversion. SSZ blocks give: (1) CL can autonomously verify the execution block hash against a builder's commitment without async EL calls; (2) binary engine API halves data exchange; (3) individual block fields become provable without the full block (light clients, wallets); (4) a cleanup opportunity to drop PoW fields and the inefficient block-level logs bloom.

## Dependencies & related EIPs
- **Depends on:** EIP-7495 (`ProgressiveContainer`), EIP-7916 (`ProgressiveList`), EIP-7773 — none of which are themselves on the Hegota candidate list, which is a real gating concern.
- **Synergizes with:** EIP-7928 (block access lists — BAL is a field of the new payload, shipped in Glamsterdam), EIP-7732 (ePBS — `ExecutionPayloadBid.block_hash`, Glamsterdam), EIP-8115 and EIP-8116 (same authors; their receipt/fee changes feed the SSZ receipts structure; 8116 makes receipts position-independent, complementing SSZ proving).
- **Conflicts / overlap with:** RLP-based tooling and any EIP assuming keccak block hashes or the current header binary format.

## Impact on ethrex / client teams
- **Large.** Touches nearly everything: block/header types (`crates/common/types/block.rs`), encoding (`crates/common/rlp` → new SSZ stack; a partial SSZ implementation exists at `crates/common/types/stateless_ssz.rs`), block-hash computation, devp2p wire protocols (`crates/networking/p2p/rlpx`), Engine API (binary encoding, dropped `block_hash`), JSON-RPC (blockHash, logsBloom), database schemas, and the BLOCKHASH opcode in the EVM.
- Forkcast's own EL-client impact note: "Major implementation work: SSZ library, header restructuring, hash changes, database and networking updates."
- Stateless/guest-program path must switch proof verification to SSZ Merkle trees.

## Open questions & controversies
- Already declined once for Hegota (acde/232) — the re-proposal signals it lost the headliner race; appetite depends on whether EL teams accept the large cross-cutting change load.
- Breaks smart contracts that verify the old header binary format or assume keccak block hashes; breaks tooling/RPC consumers broadly.
- Mixing SHA-256 `hash_tree_root` into the keccak block-hash namespace is novel (spec argues collision risk is negligible).
- Requires three prerequisite EIPs not currently in the Hegota scope.
