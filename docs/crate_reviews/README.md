# Crate Complexity & Concurrency Analyses

This directory stores the working materials for per-crate complexity and concurrency reviews: the checklist, reusable tooling, report templates, and finished analyses.

## Tracker

Update the table after each crate audit so we retain a cross-crate snapshot. Link to the rendered report in `docs/crate_reviews/reports/` and capture the high-signal metrics straight from `analyze_crate.py`.

| Crate | Commit | Date | Files / Complex Fns | Key Concurrency Signals | Report |
| --- | --- | --- | --- | --- | --- |
| `crates/networking/p2p` | `31e1950` | 2025-10-01 | 45 files / 56 functions | `.await`: 505 · `Arc<…>`: 75 · `spawn_blocking`: 11 | [ethrex_p2p_review.md](reports/ethrex_p2p_review.md) |
| `crates/blockchain` | `31e1950` | 2025-10-01 | 9 files / 15 complex | `.await`: 45 · `Arc<…>`: 3 · `spawn_blocking`: 1 | [ethrex_blockchain_review.md](reports/ethrex_blockchain_review.md) |

## Tooling

- `analyze_crate.py`: CLI helper that aggregates LOC, function complexity, and concurrency keyword stats. Example:
  ```bash
  docs/crate_reviews/analyze_crate.py "$CRATE_ROOT" --exclude dev --exclude metrics
  ```
  Pass `--json` to capture machine-readable output or `--keyword LABEL=REGEX` to extend keyword coverage.
- `_report_template.md`: Copy or reference this when writing new crate reports to keep the section structure consistent.
- `analysis_instructions.md`: Source-of-truth checklist that explains the review process end to end.

Keep related artefacts inside this directory so future updates stay discoverable.
