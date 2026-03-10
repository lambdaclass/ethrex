## Why

The ethrex release process requires ~1 person-day across 2-3 people: manual version bumps across 7 files, manual testing of 14 artifacts (8 binaries + 6 Docker images) against snap sync and L2 integration tests, manual promotion from pre-release to final, and manual Homebrew updates. Automating the full pipeline reduces release effort to two inputs (version + commit SHA) with zero manual testing or promotion steps.

## What Changes

- **New `prepare-release.yaml` workflow**: Single `workflow_dispatch` button that creates the release branch, bumps all version strings, updates lockfiles, commits, tags `vX.Y.Z-rc.1`, and pushes — triggering the existing `tag_release.yaml` build pipeline.
- **New `release-test.yaml` workflow**: Automatically tests every binary and Docker image artifact produced by `tag_release.yaml`. Runs snap sync (Hoodi, 3h timeout) for every artifact and L2 integration tests (4 variants) for L2 artifacts. Uses `fail-fast: false` so all failures are visible in one run.
- **New `promote-release.yaml` workflow**: Automatically promotes the pre-release to final when all tests pass — creates the `vX.Y.Z` tag, edits the GitHub release, and creates a PR to merge the release branch back to main.
- **New snap sync test infrastructure**: Reusable GitHub Action that runs Lighthouse + ethrex (binary or Docker) against Hoodi with `eth_syncing` polling for completion detection. No Kurtosis dependency.
- **Parameterize `docker-compose.yaml`**: Make the ethrex image tag overridable (currently hardcoded to `main`).
- **Extend `tag_latest.yaml`**: Add Homebrew tap update via `repository_dispatch` to `lambdaclass/homebrew-tap`.
- **New `update-formula.yaml` in homebrew-tap**: Automates formula version bump, sha256 computation, and bottle building.
- **Comprehensive documentation**: Update release process docs to reflect the automated pipeline.

## Capabilities

### New Capabilities
- `release-preparation`: Automated version bumping, branch creation, tagging, and lockfile updates from a single workflow_dispatch input.
- `release-testing`: Automated snap sync and L2 integration testing of all release artifacts (binaries and Docker images) with parallel execution and comprehensive failure reporting.
- `release-promotion`: Automatic promotion from pre-release to final release when all tests pass, including tag creation, release editing, and merge PR creation.
- `snap-sync-test-action`: Reusable GitHub Action for running Lighthouse + ethrex snap sync tests against a target network with polling-based completion detection.
- `homebrew-automation`: Automated Homebrew formula updates triggered by final release publication.

### Modified Capabilities
_(none — no existing spec-level behavior changes)_

## Impact

- **New files**: 3 new GitHub Actions workflows in `.github/workflows/`, 1 new composite action in `.github/actions/`, 1 new workflow in `lambdaclass/homebrew-tap`, updated release documentation.
- **Modified files**: `docker-compose.yaml` (parameterize image tag), `tag_latest.yaml` (add Homebrew dispatch step), release process docs.
- **Dependencies**: Requires self-hosted runner(s) labeled for snap sync (currently `ethrex-sync`). Expanding to arm64/macOS/GPU runners extends coverage but is not a blocker for x86_64 testing.
- **Secrets**: Uses existing `ETHREX_L1_SLACK_WEBHOOK` for failure notifications. Homebrew automation may need a PAT with cross-repo dispatch permissions.
- **External repos**: `lambdaclass/homebrew-tap` gets a new workflow.
- **Bootstrapping**: All new workflow files must be merged to `main` before the first automated release (GitHub Actions `workflow_run` constraint).
