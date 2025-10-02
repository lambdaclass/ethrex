# Crate Review Toolkit

This subdirectory contains the reusable assets for preparing a crate complexity and concurrency review. Reach for these whenever you need to run a new audit or refresh an existing report in `docs/crate_reviews/`.

## Contents
- `analysis_instructions.md`: step-by-step checklist that guides the entire review workflow.
- `analyze_crate.py`: helper script that extracts LOC, complexity, and concurrency signals. It mirrors the metrics expected in each report.
- `_report_template.md`: reusable Markdown scaffold for publishing findings.

## Usage Notes
- Run the analyzer with paths relative to the repository root, for example:
  ```bash
  docs/crate_reviews/toolkit/analyze_crate.py "crates/networking/p2p" --exclude dev
  ```
- When the write-up is ready, save or update the report Markdown in `docs/crate_reviews/` and add a new row to the tracker in the parent `README.md`.
- Keep this toolkit in sync with any new heuristics or metrics you introduce during reviews so future audits follow the same playbook.
