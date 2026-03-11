## ADDED Requirements

### Requirement: Automatic test triggering after build
The system SHALL provide a `release-test.yaml` workflow that triggers automatically when `tag_release.yaml` completes successfully for an `-rc` tag.

#### Scenario: RC tag build completes
- **WHEN** `tag_release.yaml` completes successfully for tag `v8.1.0-rc.1`
- **THEN** `release-test.yaml` triggers automatically and receives the tag name and run context

#### Scenario: Non-RC tag ignored
- **WHEN** `tag_release.yaml` completes for a non-RC tag (e.g., `v8.1.0`)
- **THEN** `release-test.yaml` does NOT trigger

#### Scenario: Main branch build ignored
- **WHEN** `tag_release.yaml` completes for a push to `main`
- **THEN** `release-test.yaml` does NOT trigger

### Requirement: Snap sync testing of every artifact
The workflow SHALL run snap sync against Hoodi for every binary and Docker image artifact produced by the release build.

The full artifact matrix:

| Artifact | Type | Platform |
|----------|------|----------|
| `ethrex-linux-x86_64` | binary | x86_64 |
| `ethrex-linux-aarch64` | binary | arm64 |
| `ethrex-macos-aarch64` | binary | macOS arm64 |
| `ethrex-l2-linux-x86_64` | binary | x86_64 |
| `ethrex-l2-linux-aarch64` | binary | arm64 |
| `ethrex-l2-macos-aarch64` | binary | macOS arm64 |
| `ethrex-l2-linux-x86_64-gpu` | binary | x86_64 + CUDA |
| `ethrex-l2-linux-aarch64-gpu` | binary | arm64 + CUDA |
| `ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.N` | Docker | x86_64 |
| `ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.N` | Docker | arm64 |
| `ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.N-l2` | Docker | x86_64 |
| `ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.N-l2` | Docker | arm64 |

Note: macOS Docker is excluded (Docker on macOS runs Linux containers, not a distinct test from Linux arm64).

#### Scenario: Binary snap sync test
- **WHEN** testing a binary artifact (e.g., `ethrex-linux-x86_64`)
- **THEN** the workflow:
  1. Downloads the binary from the GitHub pre-release
  2. Runs it natively with `--network hoodi --syncmode snap`
  3. Runs Lighthouse alongside it for consensus
  4. Uses the snap-sync-test-action to poll for completion
  5. Reports pass/fail

#### Scenario: Docker snap sync test
- **WHEN** testing a Docker image artifact (e.g., `ghcr.io/lambdaclass/ethrex:8.1.0-rc.1`)
- **THEN** the workflow:
  1. Runs `docker compose up` with the image tag overridden to the release candidate
  2. Uses the snap-sync-test-action to poll for completion
  3. Reports pass/fail

#### Scenario: Snap sync timeout
- **WHEN** snap sync does not complete within 3 hours
- **THEN** the test is marked as failed

### Requirement: L2 integration testing of L2 artifacts
The workflow SHALL run L2 integration tests for every L2 artifact (binaries and Docker images) across all test variants.

The L2 test variants: Validium, Vanilla, Web3signer, Based.

#### Scenario: L2 integration test with Docker image
- **WHEN** testing L2 Docker image `ghcr.io/lambdaclass/ethrex:8.1.0-rc.1-l2`
- **THEN** the workflow pulls the release candidate images from GHCR, runs docker compose up with L1 + L2 + contract deployer, and executes `cargo test l2_integration_test`

#### Scenario: L2 integration tests run on GitHub-hosted runners
- **WHEN** L2 integration tests are scheduled
- **THEN** they run on `ubuntu-latest` (GitHub-hosted), not on self-hosted runners, enabling full parallelism

### Requirement: All tests run regardless of failures
The workflow SHALL use `fail-fast: false` for all matrix strategies so that every test runs to completion even if other tests fail.

#### Scenario: One snap sync fails, others continue
- **WHEN** the snap sync test for `ethrex-linux-x86_64` fails
- **THEN** all other snap sync tests and L2 integration tests continue running

### Requirement: Test result aggregation and reporting
The workflow SHALL aggregate results from all tests and produce a summary.

#### Scenario: All tests pass
- **WHEN** every snap sync and L2 integration test passes
- **THEN** the aggregation job:
  1. Posts a results summary table as a comment on the GitHub pre-release
  2. Sets output `all_passed=true`

#### Scenario: Some tests fail
- **WHEN** one or more tests fail
- **THEN** the aggregation job:
  1. Posts a results summary table as a comment on the GitHub pre-release
  2. Sends a Slack notification via `ETHREX_L1_SLACK_WEBHOOK` with the list of failed tests and a link to the workflow run
  3. Sets output `all_passed=false`

### Requirement: Runner-based test scheduling
Snap sync tests SHALL run one at a time per self-hosted runner using GitHub Actions concurrency groups.

#### Scenario: Multiple snap sync tests on one runner
- **WHEN** 4 snap sync tests target the same self-hosted runner
- **THEN** they execute serially (one at a time), queuing rather than canceling
