# Updating the pinned ethrex-tooling revision

Development tooling (EF tests, load tests, monitor TUI, REPL, benchmarks, etc.)
lives in the [ethrex-tooling](https://github.com/lambdaclass/ethrex-tooling)
repository. This repo pins a single tooling commit that is used by **both**:

- the Cargo build, via the `ethrex-monitor` / `ethrex-repl` git dependencies, and
- CI, which checks `ethrex-tooling` out into `tooling/` for script/file based jobs
  (LOC report, EF tests, hive report, upgradeability, etc.).

Both must point at the **same** commit, otherwise the built binary and the CI
tooling drift apart. The procedure below keeps them in sync.

## Where the revision lives

| Location | Field | Format |
|----------|-------|--------|
| `Cargo.toml` | `rev` on `ethrex-monitor` and `ethrex-repl` | short SHA |
| `.github/actions/checkout-tooling/action.yml` | `ref` | full 40-char SHA |
| `Cargo.lock` | `ethrex-monitor` / `ethrex-repl` source | regenerated, do not hand-edit |

All CI workflows checkout via the `./.github/actions/checkout-tooling` composite
action, so the `ref` only needs to change in that one file, never in the
individual workflows.

## Procedure

1. Pick the target commit on `ethrex-tooling` (usually the tip of `main`):

   ```sh
   gh api repos/lambdaclass/ethrex-tooling/commits/main --jq .sha
   ```

2. Update `Cargo.toml` — set the `rev` on both `ethrex-monitor` and
   `ethrex-repl` to the **short** SHA (first 8 chars are enough):

   ```toml
   ethrex-monitor = { git = "https://github.com/lambdaclass/ethrex-tooling", rev = "<short-sha>" }
   ethrex-repl    = { git = "https://github.com/lambdaclass/ethrex-tooling", rev = "<short-sha>" }
   ```

3. Update `.github/actions/checkout-tooling/action.yml` — set `ref` to the
   **full** 40-char SHA (`actions/checkout` resolves a full SHA reliably; a
   short SHA may not fetch).

4. Refresh the lockfile so it records the new commit:

   ```sh
   cargo update -p ethrex-monitor -p ethrex-repl --precise <full-sha>
   ```

5. If the new tooling commit started using a **new** ethrex workspace crate,
   add a matching entry to the `[patch."https://github.com/lambdaclass/ethrex"]`
   section in `Cargo.toml` (see the comment there). Otherwise tooling's
   transitive ethrex deps will resolve to a duplicate git copy instead of the
   local workspace crates.

6. Verify:

   ```sh
   cargo check --workspace
   ```

7. Commit all of the above together (`Cargo.toml`, `Cargo.lock`,
   `.github/actions/checkout-tooling/action.yml`, and any `[patch]` change) in a
   single commit so the pin stays atomic.

## Checklist

- [ ] `Cargo.toml` `rev` bumped on both crates (short SHA)
- [ ] `checkout-tooling/action.yml` `ref` bumped (full SHA, same commit)
- [ ] `Cargo.lock` regenerated via `cargo update --precise`
- [ ] `[patch]` updated if tooling added a new workspace crate dependency
- [ ] `cargo check --workspace` passes
- [ ] all changes in one commit
