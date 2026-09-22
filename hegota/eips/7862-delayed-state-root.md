# EIP-7862: Delayed State Root
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** Proposed (CFI) (acde/240, 2026-07-02)
- **Authors:** Charlie Noyes, Dan Robinson, Justin Drake, Toni Wahrstätter
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-7862) · [discussion](https://ethereum-magicians.org/t/eip-7862-delayed-execution-layer-state-root/22559)
- **Prior fork history:** none

## TL;DR
Changes the semantics of the block header `state_root` field so block n commits to the post-state of block n-1 (its own pre-state) instead of its own post-state. This takes state root computation off the critical path between block production and attestation — validators can attest to a block without waiting for the root to be computed, and builders compute one root per slot instead of many during the MEV auction window.

## What it changes
- No new header fields; `state_root` in block n means post-state of n-1 (equivalently pre-state of n).
- `BlockChain` tracks `last_computed_state_root`; header validation checks `header.state_root == chain.last_computed_state_root` instead of recomputing after execution.
- State transition executes the block, validates all other roots (gas used, tx/receipt roots, etc.), then computes and stores the post-state root for the *next* block.
- Fork activation: at block F, `last_computed_state_root` is initialized to the post-state root of F-1; block F itself contains that root.
- Reorg rule: clients MUST recompute `last_computed_state_root` per block on the new canonical chain. Clients MUST retain the pre-state until the next block commits it (already standard practice).

## Motivation
State root computation is a major block-production bottleneck. With EIP-7732 (ePBS) tightening builder timing and EIP-7928 (BALs) enabling parallel state access, the root is the remaining serial step: BAL data can only be used to parallelize root computation if the root is due one slot later. Front-loading root computation to the start of the slot shortens the critical path to attestation.

## Dependencies & related EIPs
- Synergizes with **EIP-7928** (Block Access Lists, Glamsterdam): the BAL of block n is what makes parallelized root computation for block n feasible within slot n+1; builders can apply the BAL state diff to the slot pre-state.
- Designed for compatibility with **EIP-7732** (ePBS, Glamsterdam): only the EL header semantics change; the CL-side state_root verification in `process_execution_payload` is unaffected.
- Same author line (nerolation) and same "off the critical path" family as **EIP-8268** (storage roots in BALs), which helps partially stateful nodes verify the root.
- Related cluster EIPs: none strictly required, but 8268 complements it.

## Impact on ethrex / client teams
Touches core block validation and execution flow: header validation in `crates/blockchain/blockchain.rs` (where `validate_header` / state-root comparison live), fork-choice and reorg handling (`crates/blockchain/fork_choice.rs`), and any code that assumes header.state_root is the post-state (snap sync verification, RPC `eth_getProof` semantics, witness generation in `crates/guest-program`, Engine API payload validation in `crates/networking/rpc/engine/payload.rs`). Light-client-facing state proofs gain one slot of latency. Effort: medium — the rule itself is small, but the assumption "header root == post-state of this block" is embedded in many places (sync, proofs, tests).

## Open questions & controversies
- Repurposing the existing header field means every consumer of `state_root` (light clients, bridges, proof systems, RPC) must be updated; some prefer an explicit new field.
- One-slot delay for state proofs affects light clients and any cross-chain/bridge protocol reading fresh state proofs.
- Root availability on reorgs and "pre-state availability" requirements add bookkeeping clients must get right.
