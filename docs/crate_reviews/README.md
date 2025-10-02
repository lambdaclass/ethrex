# Crate Complexity & Concurrency Reviews

This directory hosts the finished per-crate reviews and the tracker that summarizes their core metrics.

## Tracker

Update the table after each crate audit so we retain a cross-crate snapshot. For every entry, record the effective LOC (non-empty, non-comment) and the engineering complexity score from the report, then link directly to the rendered review in this directory.

| Crate | LOC | Complexity Score | Report |
| --- | --- | --- | --- |
| `crates/blockchain` | 3,033 | 4 / 5 | [ethrex_blockchain_review.md](ethrex_blockchain_review.md) |
| `crates/common` | 8,223 | 2 / 5 | [ethrex_common_review.md](ethrex_common_review.md) |
| `crates/common/trie` | 3,904 | 3 / 5 | [ethrex_trie_review.md](ethrex_trie_review.md) |
| `crates/networking/p2p` | 13,344 | 5 / 5 | [ethrex_p2p_review.md](ethrex_p2p_review.md) |
| `crates/networking/rpc` | 8,157 | 3 / 5 | [ethrex_rpc_review.md](ethrex_rpc_review.md) |
| `crates/storage` | 5,565 | 4 / 5 | [ethrex_storage_review.md](ethrex_storage_review.md) |
| `crates/vm` | 1,121 | 2 / 5 | [ethrex_vm_review.md](ethrex_vm_review.md) |
| `crates/vm/levm` | 8,712 | 4 / 5 | [ethrex_levm_review.md](ethrex_levm_review.md) |

## Toolkit

Workflow checklists, analyzer scripts, and the report template now live under `toolkit/`. See `toolkit/README.md` for usage details when preparing a new review.
