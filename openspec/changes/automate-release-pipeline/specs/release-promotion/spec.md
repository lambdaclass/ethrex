## ADDED Requirements

### Requirement: Automatic promotion on test success
The system SHALL provide a `promote-release.yaml` workflow that triggers automatically when `release-test.yaml` completes with all tests passing.

#### Scenario: All tests pass
- **WHEN** `release-test.yaml` completes and `all_passed` is true
- **THEN** `promote-release.yaml` triggers automatically

#### Scenario: Any test fails
- **WHEN** `release-test.yaml` completes and `all_passed` is false
- **THEN** `promote-release.yaml` does NOT trigger

### Requirement: Create final release tag
The workflow SHALL create the final `vX.Y.Z` tag (without `-rc` suffix) on the same commit as the rc tag.

#### Scenario: Tag creation
- **WHEN** promoting `v8.1.0-rc.1`
- **THEN** the workflow creates tag `v8.1.0` pointing to the same commit as `v8.1.0-rc.1`

### Requirement: Edit GitHub release
The workflow SHALL update the existing GitHub pre-release to become the final release.

#### Scenario: Release promotion
- **WHEN** promoting `v8.1.0-rc.1` to `v8.1.0`
- **THEN** the workflow:
  1. Updates the release tag from `v8.1.0-rc.1` to `v8.1.0`
  2. Updates the release title to `ethrex: v8.1.0`
  3. Unchecks the "pre-release" flag
  4. Marks the release as "latest release"

#### Scenario: Release edit triggers downstream
- **WHEN** the release is edited (tag changed)
- **THEN** the existing `tag_latest.yaml` fires automatically (triggered by `release: types: [edited]`)

### Requirement: Create merge PR
The workflow SHALL create a pull request to merge the release branch back to main.

#### Scenario: PR creation
- **WHEN** promotion completes for `v8.1.0`
- **THEN** the workflow creates a PR from `release/v8.1.0` to `main` with title `release: merge v8.1.0 to main` and a description listing the release tag and link

#### Scenario: PR is not auto-merged
- **WHEN** the merge PR is created
- **THEN** the PR is left open for human review and merge — the workflow does NOT merge it
