# Hive Tests

End-to-End tests with Hive. Hive is a system which sends RPC commands to Ethereum clients and validates their responses. You can read more about it [here](https://github.com/ethereum/hive/blob/master/docs/overview.md).

## Overview

This project uses three key repositories for Hive testing:

1. **[ethereum/hive](https://github.com/ethereum/hive)** - The main Hive testing framework
   - Commit: the `hive_version` pinned in `.github/config/hive/pins.yaml`
2. **[ethereum/execution-specs](https://github.com/ethereum/execution-specs)** - Test fixtures and vectors (the former `ethereum/execution-spec-tests` repo is archived)
   - Fixtures: the release pinned in `tooling/ef_tests/.fixtures_url`, which carries every fork through Amsterdam
3. **[ethereum/execution-specs](https://github.com/ethereum/execution-specs)** - Fork specifications
   - Current branch: `forks/amsterdam`

## Prerequisites

### Required Tools

1. **Docker** - Required for running containerized clients

   ```bash
   # Install Docker following the official guide for your OS
   # Verify installation
   docker --version
   ```

2. **Go** - Required to build the Hive framework

   Using asdf:

   ```bash
   asdf plugin add golang https://github.com/asdf-community/asdf-golang.git
   ```

   Uncomment the golang line in the `.tool-versions` file:

   ```text
   rust 1.90.0
   golang 1.23.2
   ```

   Then install:

   ```bash
   asdf install
   ```

   If you need to set GOROOT, follow [these instructions](https://github.com/asdf-community/asdf-golang?tab=readme-ov-file#goroot).

3. **Rust** - Required to build the ethrex client
   ```bash
   # Already configured in .tool-versions
   asdf install rust
   ```

### Build the ethrex Docker Image

Before running Hive tests, you need to build the ethrex Docker image:

```bash
make build-image
```

## Running Hive Tests

### 1. Setup Hive Framework

The first time you run tests, Hive will be automatically cloned and built. You can also set it up manually:

```bash
make setup-hive
```

This will:

- Clone the `ethereum/hive` repository at the configured commit
- Build the Hive binary using Go
- Place it in the `./hive` directory

### 2. Basic Test Execution

#### Run Specific Simulation

Hive tests are organized by "simulations". To run a specific simulation:

```bash
make run-hive SIMULATION=<simulation-name> TEST_PATTERN="<pattern>"
```

**Available Simulations:**

- `ethereum/rpc-compat` - RPC API compatibility tests
- `devp2p` - P2P networking tests (Discovery V4, Eth, Snap)
- `ethereum/engine` - Engine API tests (Paris, Auth, Cancun, Withdrawals)
- `ethereum/sync` - Node synchronization tests
- `ethereum/eels/consume-engine` - EVM Execution Layer tests (Engine format)
- `ethereum/eels/consume-rlp` - EVM Execution Layer tests (RLP format)
- `ethereum/eels/execute-blobs` - Blob execution tests

#### Example: RPC Compatibility Tests

This is an example of a Hive simulation called `ethereum/rpc-compat`, which will specifically run chain id and transaction by hash rpc tests:

```bash
make run-hive SIMULATION=ethereum/rpc-compat TEST_PATTERN="/eth_chainId|eth_getTransactionByHash"
```

Run all RPC tests:

```bash
make run-hive SIMULATION=ethereum/rpc-compat TEST_PATTERN="*"
```

#### Run All Simulations

To run every available simulation:

```bash
make run-hive-all
```

### 3. Debug Mode

For detailed debug output including Docker container logs:

```bash
make run-hive-debug SIMULATION=ethereum/rpc-compat TEST_PATTERN="*"
```

Debug mode sets:

- `--sim.loglevel 4` (maximum verbosity)
- `--docker.output` (shows Docker container output)

### 4. Amsterdam Fork Tests (EELS)

To test Amsterdam fork compatibility using the Ethereum Execution Layer Specification tests:

#### Run Engine Format Tests

Tests all forks including Amsterdam:

```bash
make run-hive-eels-engine
```

Test specific forks:

```bash
make run-hive-eels EELS_SIM=ethereum/eels/consume-engine TEST_PATTERN_EELS=".*fork_Amsterdam.*"
```

Test multiple forks:

```bash
make run-hive-eels EELS_SIM=ethereum/eels/consume-engine TEST_PATTERN_EELS=".*fork_Prague.*|.*fork_Amsterdam.*"
```

#### Run RLP Format Tests

```bash
make run-hive-eels-rlp
```

Or test specific forks:

```bash
make run-hive-eels EELS_SIM=ethereum/eels/consume-rlp TEST_PATTERN_EELS=".*fork_Amsterdam.*"
```

#### Run Blob Execution Tests

```bash
make run-hive-eels-blobs
```

### 5. Customizing Test Execution

#### Adjust Parallelism

Control how many tests run in parallel (default: 16):

```bash
make run-hive SIMULATION=ethereum/rpc-compat SIM_PARALLELISM=8
```

#### Adjust Log Level

Set log verbosity from 1 (least verbose) to 4 (most verbose):

```bash
make run-hive SIMULATION=ethereum/rpc-compat SIM_LOG_LEVEL=1
```

#### Test Pattern Examples

Match specific test names:

```bash
TEST_PATTERN="/eth_chainId"                    # Single test
TEST_PATTERN="/eth_chainId|eth_blockNumber"    # Multiple tests
TEST_PATTERN="/eth_get.*"                       # Regex pattern
TEST_PATTERN="*"                                # All tests
```

For fork-specific EELS tests:

```bash
TEST_PATTERN_EELS=".*fork_Amsterdam.*"                                    # Amsterdam only
TEST_PATTERN_EELS=".*fork_Paris.*|.*fork_Shanghai.*|.*fork_Cancun.*"     # Multiple forks
```

## Viewing Results

### Hive Web Interface

After running tests, view results in a web interface:

```bash
make view-hive
```

This starts a local server at `http://127.0.0.1:8080` showing:

- Test pass/fail status
- Detailed logs for each test
- Client information and errors

The web interface is automatically opened after `make run-hive` completes.

### Command Line Results

Results are also available in the terminal output and in:

```
./hive/workspace/logs/
```

Log files include:

- `*.json` - Test results in JSON format
- `*.log` - Detailed execution logs
- Client-specific logs for debugging failures

### Generate Hive Report

To generate a formatted report from test results:

```bash
cargo run --manifest-path tooling/Cargo.toml -p hive_report
```

This reads all JSON files in `hive/workspace/logs/` and produces:

- Summary by category (Engine, P2P, RPC, Sync, EVM)
- Pass/fail statistics per simulation
- Fork-specific results (Paris, Shanghai, Cancun, Prague, Amsterdam)

## Cleaning Up

Remove Hive logs and workspace:

```bash
make clean-hive-logs
```

Remove the entire Hive directory to force a fresh clone:

```bash
rm -rf ./hive
```

## Repository Configuration

The project pins specific versions of the three repositories for Amsterdam fork support:

### Pin file (.github/config/hive/pins.yaml)

One file holds what the hive runs are pinned to: `tag`, the EELS simulator image tag of the fixture release; `hive_version`, the hive commit; and the pins of the two simulators that need their own, rpc-compat and consume-wirex. Each key carries a comment that says why it has the value it has.

PR CI (`pr-main_l1.yaml`) and the daily report (`daily_hive_report.yaml`) both get their hive flags and hive commit from `.github/scripts/hive-flags.sh`, which reads that file:

```bash
$ .github/scripts/hive-flags.sh --simulation ethereum/eels/consume-enginex
flags=--sim.parallelism 4 --sim.loglevel 3 --docker.buildoutput --sim.buildarg tag=<tag>
hive_version=<hive_version>
```

The root `Makefile` reads `tag` and `hive_version` from the same file, so `make setup-hive` checks out the hive commit CI runs and the local `run-hive-eels*` targets run the image CI runs. With `SIMULATION=ethereum/rpc-compat` or `SIMULATION=ethereum/eels/consume-wirex`, `make run-hive` takes that simulator's own pins instead, as CI does. Set `HIVE_BRANCH` to a branch or to another commit, by its full hash, to try a different hive.

### Fixtures URL Files

Local ef_tests runners (blockchain, state, engine) share one set of fixture URL files under `tooling/ef_tests/`:

- `tooling/ef_tests/.fixtures_url` — mainnet bundle, which from `tests@v21.0.0` on also carries the Amsterdam and BAL fixtures
- `tooling/ef_tests/.fixtures_url_zkevm` — zkEVM (EIP-8025 stateless) fixtures

There is no separate Amsterdam file any more. The mainnet bundle absorbed the `tests-glamsterdam-devnet` series and ships more Amsterdam fixtures than it did, so overlaying the devnet bundle on top would replace the richer subtrees with poorer ones.

Contents:

```
# .fixtures_url
https://github.com/ethereum/execution-specs/releases/download/tests%40v21.0.2/fixtures.tar.gz
```

`tag` in `.github/config/hive/pins.yaml` is the EELS simulator image tag of that same fixture release. The images, `ghcr.io/ethereum/execution-specs/hive/<simulator>:<tag>`, are published by execution-specs and carry that release's fixtures and framework, so hive pulls them instead of building the simulator from source; release tags such as `v21.0.1` are immutable.

## Updating Repository Versions

1. **Fixture release.** Run the bump script, which moves `tooling/ef_tests/.fixtures_url`, `tag` in the pin file and the documents that name the release, after checking that the release has a fixture bundle and that execution-specs has published the image of every simulator we run for it:

   ```bash
   .github/scripts/bump-fixtures.sh v<version> --zkevm v<zkevm version>
   ```

   A zkEVM bundle is filled from one `tests@` release, so a new base needs `--zkevm` with a zkEVM release filled from it, and the script refuses the bump without one, before it edits anything. To move only the zkEVM bundle, leave the base out:

   ```bash
   .github/scripts/bump-fixtures.sh --zkevm v<zkevm version>
   ```

   consume-wirex does not move with the bump: it keeps the image in `consume_wirex_tag`, with the fixtures that image was built from, until that pin is moved by hand.

   `make check-fixture-pins` fails when these places name different releases. It runs in PR CI.

2. **Hive commit.** Edit `hive_version` in `.github/config/hive/pins.yaml`. CI and `make setup-hive` both follow it.

3. **New forks.** If the release adds forks or networks:
   - `tooling/ef_tests/blockchain/fork.rs` and `tooling/ef_tests/engine/src/fixture.rs` - the ef-test runners' fork definitions
   - `.github/workflows/daily_hive_report.yaml` - the fork each daily job's limit selects
   - `.github/workflows/pr-main_l1.yaml` - the `fork` list of the `consume-enginex` and `consume-wirex` jobs, one job per fork: a fork that is not in it does not run in PR CI
   - `tooling/hive_report/src/main.rs` - `FORK_ORDER`, the order the report lists forks in

## Troubleshooting

### Docker Permission Errors

If you encounter Docker permission errors:

```bash
sudo usermod -aG docker $USER
newgrp docker
```

### Hive Build Failures

If Go build fails:

```bash
# Clean and rebuild
rm -rf ./hive
make setup-hive
```

### Missing Test Fixtures

If EELS tests fail due to missing fixtures:

```bash
# Verify fixtures URL is accessible
curl -I $(cat tooling/ef_tests/.fixtures_url)

# Re-download fixtures (they're downloaded automatically during test execution)
```

### Client Container Failures

If the ethrex client container fails to start:

```bash
# Rebuild the Docker image
make build-image

# Check Docker images
docker images | grep ethrex
```

## CI/CD Integration

The project runs Hive tests automatically via GitHub Actions:

- **Daily runs**: `.github/workflows/daily_hive_report.yaml` - Comprehensive test coverage
- **PR validation**: `.github/workflows/pr-main_l1.yaml` - Subset of critical tests

Results are posted to Slack and available in GitHub Actions artifacts.

## Additional Resources

- [Hive Documentation](https://github.com/ethereum/hive/blob/master/docs/overview.md)
- [Execution Specs (test fixtures)](https://github.com/ethereum/execution-specs)
- [Ethereum Execution APIs](https://github.com/ethereum/execution-apis)
- [Amsterdam Fork (Glamsterdam) Details](https://eips.ethereum.org/EIPS/eip-7928)
