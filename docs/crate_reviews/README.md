# Crate Complexity & Concurrency Analyses

This directory stores the working materials for per-crate complexity and concurrency reviews: the checklist, reusable tooling, report templates, and finished analyses.

## Tracker

Update the table after each crate audit so we retain a cross-crate snapshot. For every entry, record the effective LOC (non-empty, non-comment) and the engineering complexity score from the report, and link to the rendered artifact in `docs/crate_reviews/reports/`.

| Crate | LOC | Complexity Score | Report |
| --- | --- | --- | --- |
| `crates/blockchain` | 3,033 | 4 / 5 | [ethrex_blockchain_review.md](reports/ethrex_blockchain_review.md) |
| `crates/common` | 8,223 | 2 / 5 | [ethrex_common_review.md](reports/ethrex_common_review.md) |
| `crates/common/trie` | 3,904 | 3 / 5 | [ethrex_trie_review.md](reports/ethrex_trie_review.md) |
| `crates/networking/p2p` | 13,344 | 5 / 5 | [ethrex_p2p_review.md](reports/ethrex_p2p_review.md) |
| `crates/networking/rpc` | 8,157 | 3 / 5 | [ethrex_rpc_review.md](reports/ethrex_rpc_review.md) |
| `crates/storage` | 5,565 | 4 / 5 | [ethrex_storage_review.md](reports/ethrex_storage_review.md) |
| `crates/vm` | 1,121 | 2 / 5 | [ethrex_vm_review.md](reports/ethrex_vm_review.md) |
| `crates/vm/levm` | 8,712 | 4 / 5 | [ethrex_levm_review.md](reports/ethrex_levm_review.md) |

## Tooling

- `analyze_crate.py`: CLI helper that aggregates LOC, function complexity, and concurrency keyword stats. Example:
  ```bash
  docs/crate_reviews/analyze_crate.py "$CRATE_ROOT" --exclude dev --exclude metrics
  ```
  Pass `--json` to capture machine-readable output or `--keyword LABEL=REGEX` to extend keyword coverage.
- `_report_template.md`: Copy or reference this when writing new crate reports to keep the section structure consistent.
- `analysis_instructions.md`: Source-of-truth checklist that explains the review process end to end.

Keep related artefacts inside this directory so future updates stay discoverable.
