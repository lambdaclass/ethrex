# Crate Complexity & Concurrency Reviews

This directory hosts the finished per-crate reviews and the tracker that summarizes their core metrics.

## Tracker

| Crate | Main LOC | Test LOC | Complexity Score | Report |
| --- | --- | --- | --- | --- |
| `crates/networking/p2p` | 11,644 | 1,613 | 5 / 5 | [ethrex_p2p_review.md](ethrex_p2p_review.md) |
| `crates/vm/levm` | 7,884 | 201 | 4 / 5 | [ethrex_levm_review.md](ethrex_levm_review.md) |
| `crates/storage` | 4,619 | 912 | 4 / 5 | [ethrex_storage_review.md](ethrex_storage_review.md) |
| `crates/blockchain` | 2,419 | 598 | 4 / 5 | [ethrex_blockchain_review.md](ethrex_blockchain_review.md) |
| `crates/networking/rpc` | 6,827 | 1,284 | 3 / 5 | [ethrex_rpc_review.md](ethrex_rpc_review.md) |
| `crates/common/trie` | 2,082 | 1,837 | 3 / 5 | [ethrex_trie_review.md](ethrex_trie_review.md) |
| `crates/common` | 6,161 | 1,985 | 2 / 5 | [ethrex_common_review.md](ethrex_common_review.md) |
| `crates/vm` | 1,118 | 0 | 2 / 5 | [ethrex_vm_review.md](ethrex_vm_review.md) |

## Toolkit

Workflow checklists, analyzer scripts, and the report template now live under `toolkit/`. See `toolkit/README.md` for usage details when preparing a new review.
