# Hive Tests

End-to-End tests with Hive. Hive is a system which sends RPC commands to Ethereum clients and validates their responses. You can read more about it [here](https://github.com/ethereum/hive/blob/master/docs/overview.md).

## Overview

This project uses three key repositories for Hive testing:

1. **[ethereum/hive](https://github.com/ethereum/hive)** - The main Hive testing framework
    - Current commit: `0921fb7833e3de180eacdc9f26de6e51dcab0dba`
2. **[ethereum/execution-spec-tests](https://github.com/ethereum/execution-spec-tests)** - Test fixtures and vectors
    - Current version: `bal@v5.1.0` (Amsterdam fork support)
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

Run specific RPC endpoints:

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

### Makefile Configuration

```makefile
# ethereum/hive branch
HIVE_BRANCH ?= master
```

### Workflow Configuration (.github/workflows/daily_hive_report.yaml)

The workflow uses fork-specific fixtures to ensure comprehensive test coverage:

```yaml
# Amsterdam tests use fixtures_bal (includes BAL-specific tests)
if [[ "$SIM_LIMIT" == *"fork_Amsterdam"* ]]; then
  FLAGS+=" --sim.buildarg fixtures=https://github.com/ethereum/execution-spec-tests/releases/download/bal@v5.1.0/fixtures_bal.tar.gz"
  FLAGS+=" --sim.buildarg branch=forks/amsterdam"
else
  # Other forks use fixtures_develop (comprehensive coverage including static tests)
  FLAGS+=" --sim.buildarg fixtures=https://github.com/ethereum/execution-spec-tests/releases/download/v5.3.0/fixtures_develop.tar.gz"
  FLAGS+=" --sim.buildarg branch=forks/osaka"
fi
```

### Fixtures URL Files

- `tooling/ef_tests/blockchain/.fixtures_url`
- `tooling/ef_tests/state/.fixtures_url`

Both contain:

```
https://github.com/ethereum/execution-spec-tests/releases/download/bal@v5.1.0/fixtures_bal.tar.gz
```

**Note**: The workflow uses both `fixtures_bal` (for Amsterdam) and `fixtures_develop` (for other forks) to ensure complete multi-fork compatibility.

## Updating Repository Versions

To update to a different fork or newer versions:

1. **Update Hive commit** in `Makefile`:

    ```makefile
    HIVE_BRANCH ?= <new-commit-hash>
    ```

2. **Update execution-spec-tests versions** in `.github/workflows/daily_hive_report.yaml`:

    For Amsterdam tests (fixtures_bal):

    ```yaml
    FLAGS+=" --sim.buildarg fixtures=https://github.com/ethereum/execution-spec-tests/releases/download/bal@<version>/fixtures_bal.tar.gz"
    FLAGS+=" --sim.buildarg branch=forks/amsterdam"
    ```

    For other forks (fixtures_develop):

    ```yaml
    FLAGS+=" --sim.buildarg fixtures=https://github.com/ethereum/execution-spec-tests/releases/download/v<version>/fixtures_develop.tar.gz"
    FLAGS+=" --sim.buildarg branch=forks/<fork-name>"
    ```

3. **Update fixtures URL files**:

    ```bash
    echo "https://github.com/ethereum/execution-spec-tests/releases/download/bal@<version>/fixtures_bal.tar.gz" > tooling/ef_tests/blockchain/.fixtures_url
    echo "https://github.com/ethereum/execution-spec-tests/releases/download/bal@<version>/fixtures_bal.tar.gz" > tooling/ef_tests/state/.fixtures_url
    ```

4. **Update fork references** in code if switching to a different fork:
    - `.github/workflows/daily_hive_report.yaml` - Test names and patterns
    - `tooling/hive_report/src/main.rs` - Fork ranking and result processing

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
curl -I $(cat tooling/ef_tests/blockchain/.fixtures_url)

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
- [Execution Spec Tests](https://github.com/ethereum/execution-spec-tests)
- [Ethereum Execution APIs](https://github.com/ethereum/execution-apis)
- [Amsterdam Fork (Glamsterdam) Details](https://eips.ethereum.org/EIPS/eip-7928)
