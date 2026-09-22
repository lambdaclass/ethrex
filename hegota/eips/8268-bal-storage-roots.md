# EIP-8268: Storage Roots in Block Access Lists
- **Layer:** EL
- **EIP status:** Draft
- **Hegota status:** none recorded (forkcast shows no Hegota forkRelationship)
- **Authors:** Toni Wahrstätter, Carlos Perez
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-8268) · [discussion](https://ethereum-magicians.org/t/eip-8268-storage-roots-in-block-access-lists/28585)
- **Prior fork history:** none

## TL;DR
Extends EIP-7928 Block Access Lists so that every `AccountChanges` entry with any state modification also carries the account's post-block storage trie root. This lets a node holding only a partition of the state reconstruct the full state-trie leaf for every modified account and verify the post-block state root — the one piece of post-state not derivable from BAL diffs alone.

## What it changes
- `AccountChanges` gains a trailing 7th field `storage_root: bytes32` — present **only** when at least one of `storage_changes`, `balance_changes`, `nonce_changes`, `code_changes` is non-empty. Accessed-but-unchanged entries keep the 6-field EIP-7928 layout.
- Empty storage tries (the common case: EOAs and storageless accounts that only changed balance/nonce) MUST encode as the empty byte string (RLP `0x80`), never the canonical 32-byte empty-trie root `0x56e81f…`; consumers treat `0x80` as the canonical empty root when rebuilding the leaf. The rule keys on post-block storage-trie emptiness, not account type, so it covers EIP-7702 delegated accounts and newly created/cleared contracts.
- Validation: a block is invalid if any `storage_root` mismatches the actual post-block storage trie root (or the encoding rule above). Each block has exactly one valid BAL encoding; `storage_root` is committed by `block_access_list_hash`.
- Overhead: up to 32 bytes per modified account, zero for touched-only accounts.

## Motivation
A BAL gives per-slot post-values plus balance/nonce/code diffs, but an account's storage root depends on its *entire* storage trie — unknowable to a partially stateful node that doesn't store that account's untouched slots. Without it, such nodes cannot assemble the state-trie leaf and cannot verify the post-block state root. Adding the root closes this gap for state-partitioned / partially stateful node designs.

## Dependencies & related EIPs
- Requires **EIP-7928** (Block Access Lists, scheduled in Glamsterdam) — this is a strict extension of the BAL encoding; the two encodings are not interchangeable.
- Synergizes with **EIP-7862** (Delayed State Root, same champion nerolation): delayed roots plus verifiable per-account storage roots are both aimed at decoupling state-root work from block validation and enabling partial-state validation.
- Relevant to EIP-7702-style delegated accounts (Pectra) via the empty-trie encoding rule.

## Impact on ethrex / client teams
ethrex already implements EIP-7928 BALs end-to-end (`crates/common/types/block_access_list.rs` — `AccountChanges` is exactly the 6-field struct this EIP extends; RLP codec, `block_access_list_hash`, validation in `crates/blockchain` and Engine API encoding in `crates/networking/rpc`). The change is: add the optional 7th field, compute per-account post-block storage roots during execution (requires the account's full post-storage trie — free for full nodes, it is computed anyway), and add the canonical empty-trie encoding rule to BAL validation. Touches BAL generation, validation, RLP, RPC/engine encoding. Effort: small-to-medium (well-scoped, but consensus-critical codec change).

## Open questions & controversies
- No Hegota forkRelationship recorded on forkcast — may simply not have been formally proposed yet (created 2026-05).
- The mandatory `0x80` empty-trie encoding is a consensus-critical special case every client must match exactly; a natural source of cross-client consensus bugs.
- Depends on BALs (EIP-7928) shipping and stabilizing first; value is mostly unrealized until partially stateful node designs actually exist.
