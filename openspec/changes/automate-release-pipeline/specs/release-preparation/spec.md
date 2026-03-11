## ADDED Requirements

### Requirement: Single-button release initiation
The system SHALL provide a `workflow_dispatch`-triggered GitHub Actions workflow (`prepare-release.yaml`) that accepts a release version and a commit SHA as inputs, and produces a tagged release candidate ready for the existing build pipeline.

#### Scenario: Successful release preparation
- **WHEN** a maintainer triggers `prepare-release.yaml` with version `8.1.0` and commit `abc123`
- **THEN** the workflow:
  1. Checks out commit `abc123`
  2. Creates branch `release/v8.1.0` from that commit
  3. Updates the version string to `8.1.0` in all required files
  4. Runs `make update-cargo-lock` to synchronize lockfiles
  5. Creates a commit with all version changes
  6. Creates tag `v8.1.0-rc.1`
  7. Pushes the branch and tag to origin

#### Scenario: Tag push triggers build pipeline
- **WHEN** `prepare-release.yaml` pushes tag `v8.1.0-rc.1`
- **THEN** the existing `tag_release.yaml` workflow fires automatically (it matches the `v*.*.*-*` pattern)

### Requirement: Version bump covers all required files
The workflow SHALL update the version string in exactly these files:

| File | Field |
|------|-------|
| `Cargo.toml` (workspace root) | `workspace.package.version` |
| `crates/guest-program/bin/sp1/Cargo.toml` | `package.version` |
| `crates/guest-program/bin/risc0/Cargo.toml` | `package.version` |
| `crates/guest-program/bin/zisk/Cargo.toml` | `package.version` |
| `crates/guest-program/bin/openvm/Cargo.toml` | `package.version` |
| `crates/l2/tee/quote-gen/Cargo.toml` | `package.version` |
| `docs/CLI.md` | All occurrences of `"ethrex <old_version>"` → `"ethrex <new_version>"` |

#### Scenario: All files updated correctly
- **WHEN** prepare-release runs with version `8.1.0` on a codebase at version `8.0.0`
- **THEN** all 7 files contain `8.1.0` in their respective version fields, and `CLI.md` references `"ethrex 8.1.0"` instead of `"ethrex 8.0.0"`

#### Scenario: Lockfiles synchronized
- **WHEN** version bumps are committed
- **THEN** `make update-cargo-lock` has been run and the resulting lockfile changes are included in the commit

### Requirement: Input validation
The workflow SHALL validate inputs before making changes.

#### Scenario: Invalid version format
- **WHEN** the version input does not match the pattern `X.Y.Z` (three integers separated by dots)
- **THEN** the workflow fails immediately with a clear error message

#### Scenario: Invalid commit SHA
- **WHEN** the commit SHA does not exist in the repository
- **THEN** the workflow fails immediately with a clear error message

#### Scenario: Release branch already exists
- **WHEN** branch `release/vX.Y.Z` already exists
- **THEN** the workflow fails immediately with an error indicating the branch already exists
