## ADDED Requirements

### Requirement: Homebrew update triggered by release
The system SHALL automatically update the Homebrew formula in `lambdaclass/homebrew-tap` when a final release is published.

#### Scenario: Dispatch from tag_latest
- **WHEN** `tag_latest.yaml` completes the Docker retag and apt publish
- **THEN** it dispatches a `repository_dispatch` event to `lambdaclass/homebrew-tap` with the release version as payload

### Requirement: Automated formula update
The `lambdaclass/homebrew-tap` repository SHALL have a workflow (`update-formula.yaml`) that updates the `ethrex.rb` formula on `repository_dispatch`.

#### Scenario: Formula version bump
- **WHEN** `update-formula.yaml` receives a dispatch with version `8.1.0`
- **THEN** the workflow:
  1. Downloads the source tarball from `https://github.com/lambdaclass/ethrex/archive/refs/tags/v8.1.0.tar.gz`
  2. Computes the SHA-256 hash of the tarball
  3. Updates `Formula/ethrex.rb` with the new URL and SHA-256
  4. Builds a bottle for macOS arm64 (Sonoma)
  5. Updates the bottle SHA-256 in the formula
  6. Commits the formula changes
  7. Creates a release `v8.1.0` in the homebrew-tap repo with the bottle attached

### Requirement: tag_latest extension
The `tag_latest.yaml` workflow SHALL be extended with a new job that dispatches to the homebrew-tap repo.

#### Scenario: Dispatch step
- **WHEN** the `retag_docker_images` and `publish-apt` jobs complete
- **THEN** a new `update-homebrew` job sends a `repository_dispatch` event with `event_type: update-formula` and payload `{ "version": "<version>" }` to `lambdaclass/homebrew-tap`

#### Scenario: PAT secret required
- **WHEN** the dispatch step runs
- **THEN** it uses a secret (e.g., `HOMEBREW_DISPATCH_TOKEN`) with `actions:write` permission on `lambdaclass/homebrew-tap`
