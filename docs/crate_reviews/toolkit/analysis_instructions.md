# Crate Complexity & Concurrency Analysis Checklist

Use these steps when preparing a complexity-focused review for an `ethrex` crate (or a similar Rust workspace component). Adjust paths and filters as needed. Throughout this document, treat `$CRATE_ROOT` as the path to the crate under analysis (for example, `crates/networking/p2p`). Some crate directories include additional nested crates (e.g., sibling `dev/` or `metrics/` packages); make sure your analysis only covers the target crate’s own sources.

`ethrex` increasingly relies on the [`spawned`](https://github.com/lambdaclass/spawned) actor framework (Erlang-style `GenServer`s and message channels). When you see paired mutex/state structures, check whether they predate the actor migration and call them out for alignment with the actor model.

**Before you start**
- Capture the commit under review so the final report can reference it:
  ```bash
  git rev-parse HEAD
  ```
- Note the current date/time and any relevant feature flags or build settings supplied by the requester.
- Deliverable: commit your findings to a new or updated report under `docs/crate_reviews/`, using `toolkit/_report_template.md` as the structure.


## 1. Scope the Target Crate
- Locate the crate directory and set `$CRATE_ROOT` accordingly.
- Confirm which subdirectories must be excluded. Only skip `dev/`, `metrics/`, or similar directories when they contain separate crates that are **not** part of the current review scope.
- Verify the crate’s entry point and dependency list via `Cargo.toml`.

## 2. Inventory Source Files
- List Rust source files in scope, skipping excluded subtrees. Example when you need to ignore sibling crates:
  ```bash
  rg --files -g'*.rs' -g'!other-crate/**' "$CRATE_ROOT"
  ```
- For crates without nested subcrates simply omit the exclusion patterns.
- Capture line counts with `cargo warloc` so you can separate production and test code:
  - Run the tool from the crate root (for example `cargo warloc --by-file`) to dump per-file totals.
  - Identify subdirectories that are standalone crates (any directory with its own `Cargo.toml`) and discard their rows before you add things up; `cargo warloc` includes the entire folder tree.
  - Aggregate the filtered numbers into the table printed by the tool (skip the `Examples` row) and stash it for the write-up.
- Flag inline test modules (`#[cfg(test)]`) or dedicated test files; you may keep their metrics separate when they skew results (e.g., large fixtures) and explicitly note when production totals exclude files such as `*_test.rs`.

## 3. Function-Level Metrics
- Count total functions and identify “complex” ones. A practical heuristic:
  - Body length ≥ 60 lines, or
  - ≥ 6 branch keywords (`if`, `match`, loops, logical chains), or
  - Length ≥ 40 lines **and** ≥ 3 branches.
- Prefer the reusable helper script to gather stats:
  ```bash
  docs/crate_reviews/toolkit/analyze_crate.py "$CRATE_ROOT"
  ```
  Pass `--exclude <dir>` only when the crate directory contains sibling crates you need to drop (e.g., `--exclude dev`). Use `--exclude-prefix <path>` when you need to skip a nested crate subtree (for example `--exclude-prefix levm` while keeping `backends/levm`). The script emits file totals, complex function candidates, and concurrency keyword counts. Extend the search surface with `--keyword LABEL=REGEX` if you need crate-specific signals.
- If you need raw data for spreadsheets, add `--json > crate_analysis.json` and import the output into your tooling of choice.

## 4. Concurrency & Blocking Signals
- Use the helper script’s keyword output as a starting point. By default it tracks `async fn`, `.await`, spawns, locks, atomics, channels, actor usage, `MutexGuard` + `.await`, and `Arc<Mutex<mpsc::Receiver<_>>>` bridges. Add or adjust patterns with `--keyword` as needed, and sanity-check surprising totals with a targeted `rg` sweep (e.g., count both `tokio::spawn` and `tokio::task::spawn`).
- When you need line-level context, fall back to focused ripgrep queries:
  ```bash
  rg -n "tokio::sync::RwLock" "$CRATE_ROOT"
  rg -n "Arc<Mutex<mpsc::Receiver" "$CRATE_ROOT"
  rg -n "\.await" "$CRATE_ROOT" | wc -l
  ```
- Summarize counts per file to spot hotspots. The script already surfaces top files per keyword; capture anything surprising in your notes.
- When scanning `.await` sites, check whether they occur while holding a synchronous guard (`MutexGuard`, `RwLockWriteGuard`, etc.) and call those out explicitly as potential deadlocks or priority inversions.
- Call out patterns where blocking IO (`std::fs`, heavy database calls) appears inside async contexts without `spawn_blocking` or dedicated worker tasks.
- Remember to check for Clippy warnings such as `cargo clippy -W clippy::await_holding_lock` when you suspect lock/await interactions.

## 5. Qualitative Review
- Read through the longest/most complex functions identified in step 3.
- Focus on sections that combine long lock hold times, async `.await` points inside locks, or nested branching around stateful operations.
- Note where `Arc<Mutex<_>>`, `Arc<TokioMutex<_>>`, or `Arc<Mutex<mpsc::Receiver<_>>>` bridge blocking and async code—these often serialize peer or network workflows.
- Inspect how shared maps (e.g., peer tables) are guarded; coarse `Arc<Mutex<_>>` structures may merit sharding or alternative primitives (`DashMap`, `RwLock`).
- Look for actor components (`GenServer`, message `Cast`/`Call` patterns). Document how state flows through actors, whether they still rely on interior mutexes, and highlight opportunities to replace remaining locks with actor messages for consistency.

## 6. Context Gathering
- Inspect `Cargo.toml` to understand feature flags and optional subcrates (e.g., metrics, dev tooling) that should remain out of scope.
- Use `cargo metadata --no-deps` to gauge dependency fan-in/out if needed.

## 7. Synthesize Findings
- Capture quantitative metrics: file count, LOC (include the `cargo warloc` table), function totals, complex function tally, concurrency/blocking keyword counts.
- Highlight top risky areas with file + line references (long functions, mixed locking patterns, async hotspots).
- Assign a 1–5 engineering risk/complexity score based on size, branching density, concurrency surface, and critical-path functions.
- Include the commit hash and date captured earlier near the top of the write-up so future reviews know exactly what was analyzed. Start from the template in `docs/crate_reviews/toolkit/_report_template.md` so reports stay consistent.
- Before publishing, run `docs/crate_reviews/toolkit/linkify_report_refs.py <report.md>` to translate plain `path.rs:123` mentions into GitHub permalinks anchored at the commit you recorded. The script is idempotent, so rerun it whenever you update an existing report.
- Suggest actionable next steps (e.g., refactoring targets, instrumentation ideas, lock/actor strategy adjustments, migrations away from mutexes toward actors where appropriate). Log results in the shared tracker if one exists for cross-crate comparisons.

## 8. Optional Enhancements
- Run `cargo geiger`, `cargo llvm-lines`, or `cargo +nightly fmt -- --check` / `cargo clippy` profiles to augment the static signal set when time allows.
- Profile lock contention (e.g., `tokio-console`, tracing instrumentation) when runtime data is needed.
- Track new learnings per crate analysis (e.g., patterns like large inline fixtures, use of `spawn_blocking` around DB access) and fold them back into this checklist to keep it fresh.

Keep this checklist updated as new tooling or heuristics prove useful.
