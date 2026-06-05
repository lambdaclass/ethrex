# Updating the pinned ethrex-tooling revision

Development tooling (EF tests, load tests, monitor TUI, REPL, benchmarks, etc.)
lives in the [ethrex-tooling](https://github.com/lambdaclass/ethrex-tooling)
repository. This repo pins a single tooling commit that is used by **both**:

- the Cargo build, via the `ethrex-monitor` / `ethrex-repl` git dependencies, and
- CI, which checks `ethrex-tooling` out into `tooling/` for script/file based jobs
  (LOC report, EF tests, hive report, upgradeability, etc.).

Both must point at the **same** commit, otherwise the built binary and the CI
tooling drift apart. The procedure below keeps them in sync.

## Local development

Targets that use the physical tooling files (`make load-test`,
`make sort-genesis-files`, etc.) expect `ethrex-tooling` checked out at
`./tooling`. Set it up with:

```sh
make setup-tooling    # clone (if needed) and check out the pinned rev
make verify-tooling   # check ./tooling matches the rev pinned in Cargo.toml
```

`setup-tooling` checks out the exact rev pinned in `Cargo.toml`, so local builds
match CI. These targets fail fast with a pointer to this doc if `./tooling` is
missing, and `verify-tooling` reports a mismatch if the checkout drifts from the
pin (e.g. after a `rev` bump — re-run `make setup-tooling` to sync).

## Where the revision lives

| Location | Field | Format |
|----------|-------|--------|
| `Cargo.toml` | `rev` on `ethrex-monitor` and `ethrex-repl` | short SHA |
| `.github/workflows/*.yaml` | `env.TOOLING_REV` (the `ref` of each ethrex-tooling checkout) | full 40-char SHA |
| `Cargo.lock` | `ethrex-monitor` / `ethrex-repl` source | regenerated, do not hand-edit |

Each workflow defines `TOOLING_REV` once at the top and every ethrex-tooling
checkout uses `ref: ${{ env.TOOLING_REV }}`. A plain `actions/checkout` (not a
local composite action) is used on purpose: some jobs check out tooling before /
without the ethrex repo tree (and on the base ref), where a local action would
not yet exist.

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

3. Update `env.TOOLING_REV` in every workflow under `.github/workflows/` to the
   **full** 40-char SHA (`actions/checkout` resolves a full SHA reliably; a
   short SHA may not fetch). They all hold the same value, so:

   ```sh
   sed -i -E 's/(  TOOLING_REV: ).*/\1<full-sha>/' .github/workflows/*.yaml
   ```

4. Refresh the lockfile so it records the new commit:

   ```sh
   cargo update -p ethrex-monitor -p ethrex-repl --precise <full-sha>
   ```

5. If the new tooling commit started using a **new** ethrex workspace crate,
   add a matching entry to the `[patch."https://github.com/lambdaclass/ethrex"]`
   section in `Cargo.toml` (see the comment there). Otherwise tooling's
   transitive ethrex deps will resolve to a duplicate git copy instead of the
   local workspace crates. CI enforces this via
   `.github/scripts/check_tooling_patch.sh` (run in the L1 lint job); run it
   locally to check:

   ```sh
   bash .github/scripts/check_tooling_patch.sh
   ```

6. Verify:

   ```sh
   cargo check --workspace
   ```

7. Commit all of the above together (`Cargo.toml`, `Cargo.lock`,
   `.github/workflows/*.yaml`, and any `[patch]` change) in a single commit so
   the pin stays atomic.

## Checklist

- [ ] `Cargo.toml` `rev` bumped on both crates (short SHA)
- [ ] `env.TOOLING_REV` bumped in every `.github/workflows/*.yaml` (full SHA, same commit)
- [ ] `Cargo.lock` regenerated via `cargo update --precise`
- [ ] `[patch]` updated if tooling added a new workspace crate dependency
- [ ] `cargo check --workspace` passes
- [ ] all changes in one commit
