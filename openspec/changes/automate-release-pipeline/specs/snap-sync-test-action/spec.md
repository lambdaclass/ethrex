## ADDED Requirements

### Requirement: Reusable snap sync test action
The system SHALL provide a reusable GitHub composite action (`.github/actions/snap-sync-test/action.yml`) that runs an ethrex node with Lighthouse against a target network and verifies sync completion.

#### Scenario: Binary mode
- **WHEN** the action is invoked with `mode: binary` and a path to an ethrex binary
- **THEN** the action:
  1. Generates a JWT secret
  2. Starts Lighthouse with checkpoint sync for the target network
  3. Starts the ethrex binary with `--syncmode snap --network <network> --authrpc.jwtsecret <jwt>`
  4. Polls `eth_syncing` for completion
  5. Cleans up both processes on completion or timeout

#### Scenario: Docker mode
- **WHEN** the action is invoked with `mode: docker` and an image tag
- **THEN** the action:
  1. Overrides the ethrex image tag in `docker-compose.yaml` via the `ETHREX_TAG` environment variable
  2. Runs `docker compose up -d`
  3. Polls `eth_syncing` for completion
  4. Runs `docker compose down` on completion or timeout

### Requirement: Two-phase sync completion detection
The action SHALL detect sync completion using a two-phase polling approach on the `eth_syncing` JSON-RPC endpoint.

#### Scenario: Phase 1 — wait for sync to start
- **WHEN** the action starts polling
- **THEN** it waits until `eth_syncing` returns a syncing object with `highestBlock > 0`, retrying on connection errors and `false` returns (which indicate "not started yet")

#### Scenario: Phase 2 — wait for sync to complete
- **WHEN** phase 1 detects sync has started
- **THEN** it waits until `eth_syncing` returns `false` (meaning sync is complete)

#### Scenario: Validation after sync
- **WHEN** `eth_syncing` returns `false` after having been in a syncing state
- **THEN** the action calls `eth_blockNumber` and verifies the block number is greater than 0

### Requirement: Configurable timeout
The action SHALL accept a timeout parameter and fail the test if sync does not complete within that time.

#### Scenario: Timeout exceeded
- **WHEN** the timeout (default 3 hours) elapses before sync completes
- **THEN** the action terminates all processes, collects logs from both Lighthouse and ethrex, and exits with a non-zero code

#### Scenario: Log collection on failure
- **WHEN** the test fails (timeout or crash)
- **THEN** the action outputs the last 200 lines of ethrex logs and the last 100 lines of Lighthouse logs to the GitHub Actions step summary

### Requirement: Action inputs
The action SHALL accept these inputs:

| Input | Required | Default | Description |
|-------|----------|---------|-------------|
| `mode` | yes | — | `binary` or `docker` |
| `binary_path` | if mode=binary | — | Path to the ethrex binary |
| `image_tag` | if mode=docker | — | Docker image tag (e.g., `8.1.0-rc.1`) |
| `network` | no | `hoodi` | Target network |
| `timeout` | no | `3h` | Maximum time to wait for sync |
| `lighthouse_version` | no | `v8.0.1` | Lighthouse version to download |
| `poll_interval` | no | `60` | Seconds between `eth_syncing` polls |

#### Scenario: Defaults applied
- **WHEN** the action is invoked with only `mode` and `binary_path`
- **THEN** it uses `hoodi` as the network, `3h` as the timeout, `v8.0.1` as the Lighthouse version, and 60s as the poll interval
