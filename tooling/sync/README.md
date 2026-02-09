# Quick starter guide to sync tooling

The targets provided by the makefile aim towards making starting a sync or running a benchmark on Ethrex much simpler. This readme will provide a quick explanation to get you started.

## Environment variables

The commands use a number of environment variables, which can be easily passed alongside the `make` command to provide some settings to the target being run. Many of the commands *will not run* if requisite environment variables aren't set. These variables are:

- `NETWORK`: network on which to sync (at the moment, only mainnet, sepolia, holesky and hoodi are supported as options). If this variable is not set `mainnet` will be used by default.

- `EVM`: the EVM which will be used. `levm` is the default, but it can be set to `revm` as well.

- `LOGNAME`: used in the flamegraph commands to append a custom naming to the default name scheme, and in the tailing commands to select the log file to tail.

- `SYNC_BLOCK_NUM`: block number on which to start the sync. Required by both the `sync` and `flamegraph` commands. All the commands which use this variable require it to be set by the user.

- `EXECUTE_BATCH_SIZE`: the amount of blocks to execute in batch during full sync. Optional.

- `BRANCH`: required by the `flamegraph-branch` command. Branch on which to run.

- `GRAPHNAME`: used by the `copy-flamegraph` command to provide a custom name to the flamegraph being copied.

## Logs

All logs are output to the `logs` folder in `tooling/sync`. The sync logs follow the naming convention `ethrex-sync-NETWORK-EVM.log` (replacing NETWORK and EVM with the network and evm being used), whereas all the flamegraph logs follow the naming convention `ethrex-NETWORK-EVM-flamegraph-CURRENT_DATETIME-BRANCH-block-BLOCK_NUM-LOGNAME.log`, with CURRENT_DATETIME being the date and time the run was started in in the format YY.MM.DD-HH.MM.SS, BRANCH being the ethrex repository branch the run was done on, and SYNC_BLOCK_NUM being the block the sync was started on.

## Database location

The databases are stored in the `~/.local/share/` folder in Linux, and `~/Library/Application Support` in Mac. For each network, a NETWORK_data folder is created. Inside this folder is the jwt our command creates, and an `ethrex` folder; which will contain one EVM folder for each evm ethrex was ran with on the network that corresponds to the current path (so, for example, if a sync was run with levm on hoodi, a `~/.local/share/hoodi_data/ethrex/levm` folder will be present. Then, if another sync in hoodi is run with revm, a `~/.local/share/hoodi_data/ethrex/revm` will be created).

## Running a sync

Lighthouse must be running for the sync to work. Aditionally, a jwt has to be provided too. The SYNC_BLOCK_NUM also has to be one a batch ended on for that network and evm. *The sync will not work if not started from a block number like such*, so it's important to check the numebr carefully.

## Running flamegraphs

You will first need to install flamegraph by running:

```=bash
cargo install flamegraph
```

It's advisable to only run flamegraphs on blocks that have already been synced, so that the overhead of retrieving the headers and bodies from the network doesn't distort the measurements. The generated flamegraphs are stored by default in the ethrex root folder. You can run the flamegraph using the provided commands. The run has to be stopped manually interrupting it with `ctrl + c`. Afterwards, a script starts that creates a flamegraph from the gathered data. Once this script finishes, the flamegraph should be ready.

## Commands

- `make gen_jwt` generates the jwt to use to connect to the network. `NETWORK` must be provided. 

- `make sync` can be used to start a sync. `NETWORK` and `SYNC_BLOCK_NUM` must be provided, `EVM` can be optionally provided too.

- `make flamegraph-main` and `make flamegraph-branch` can be used to run benchmarks on the main branch of the repo or a custom branch, respectively; generating both a flamegraph and logs of the run. `NETWORK` and `SYNC_BLOCK_NUM` must be provided, `EVM` can be optionally provided too. `BRANCH` must be provided for `flamegraph-branch` as well. `make flamegraph` can also be used as a branch agnostic option.

- `make start-lighthouse` can be used to start lighthouse. `NETWORK` must be provided or else mainnet will be used as default.

- `make backup-db` can be used to create a backup of the database. `NETWORK` must be provided, and `EVM` should be provided too. Backups are stored in `~/.local/share/ethrex_db_backups` in Linux and `~/Library/Application Support/ethrex_db_backups` folder in MacOS. The logs up to that point are also backed up in the same folder.

- `make tail-syncing-logs` can be used to easily tail the syncing information in any given log. `LOGNAME` must be provided to indicate the log file to tail.

- `make tail-metrics-logs` can be used to easily tail the metrics information in any given log (how long batches are taking to process). `LOGNAME` must be provided to indicate the log file to tail.

- `make copy-flamegraph` can be used to quickly copy the flamegraph generated by the flamegraph commands from the `ethrex` repo folder to the `tooling/sync/flamegraphs` folder so it isn't overwritten by future flamegraph runs. `GRAPHNAME` can be provided to give the file a custom name.

- `make import-with-metrics` can be used to import blocks from an RLP file with metrics enabled, specially useful for a block processing profile. The path to the rlp file can be passed with the `RLP_FILE` environment variable, while the network can be provided with the `NETWORK` variable.

## Multi-Network Parallel Snapsync

This feature allows running multiple Ethrex nodes in parallel (hoodi, sepolia, mainnet) via Docker Compose, with automated monitoring, Slack notifications, and a history log of runs.

### Overview

The parallel snapsync system:
- Spawns multiple networks simultaneously via Docker Compose
- Monitors snapsync progress with configurable timeout (default 8 hours)
- Verifies block processing after sync completion (default 22 minutes)
- Sends Slack notifications on success/failure
- Maintains a history log of all runs
- On success: restarts containers and begins a new sync cycle
- On failure: keeps containers running for debugging

### Auto-Update Mode with State Trie Validation

The `multisync-loop-auto` target provides continuous integration testing by:
1. **Pulling latest code** from a configured branch before each run
2. **Building Docker image** with configurable Cargo profile
3. **Running state trie validation** when using `release-with-debug-assertions` profile
4. **Looping continuously** on success, stopping on failure for inspection

State trie validation (enabled with `release-with-debug-assertions` profile) verifies:
- **State root**: Traverses entire account trie, validates all node hashes
- **Storage roots**: Validates each account's storage trie (parallelized)
- **Bytecodes**: Verifies code exists for all accounts with code

This mirrors the daily snapsync CI checks but runs continuously on your own infrastructure.

**Quick Start:**

```bash
# Run with validation on current branch
make multisync-loop-auto

# Run on specific branch
make multisync-loop-auto MULTISYNC_BRANCH=main

# Run without validation (faster builds)
make multisync-loop-auto MULTISYNC_BUILD_PROFILE=release
```

**Configuration (in `.env` or as make variables):**

| Variable | Default | Description |
|----------|---------|-------------|
| `MULTISYNC_BRANCH` | current branch | Git branch to track |
| `MULTISYNC_BUILD_PROFILE` | `release-with-debug-assertions` | Cargo build profile |
| `MULTISYNC_LOCAL_IMAGE` | `ethrex-local:multisync` | Docker image tag |
| `MULTISYNC_NETWORKS` | `hoodi,sepolia,mainnet` | Networks to sync |

**Run count persistence:** The run count is persisted across restarts by reading from the history log. If run #5 fails and you restart, the next run will be #6.

### Requirements

- Docker and Docker Compose
- Python 3 with the `requests` library (`pip install requests`)
- (Optional) Slack webhook URLs for notifications

### Quick Start

```bash
# Start a continuous monitoring loop (recommended for servers)
make multisync-loop

# Or run a single sync cycle
make multisync-run
```

### Docker Compose Setup

The `docker-compose.multisync.yaml` file defines services for each network with isolated volumes. Each network uses Lighthouse as the consensus client with checkpoint sync.

Host port mapping:
- **hoodi**: `localhost:8545`
- **sepolia**: `localhost:8546`
- **mainnet**: `localhost:8547`
- **hoodi-2**: `localhost:8548` (for additional testing)

### Environment Variables

Create a `.env` file in `tooling/sync/` with:

```bash
# Slack notifications (optional)
SLACK_WEBHOOK_URL_SUCCESS=https://hooks.slack.com/services/...
SLACK_WEBHOOK_URL_FAILED=https://hooks.slack.com/services/...

# Monitoring timeouts (optional - values shown are defaults)
SYNC_TIMEOUT=480                  # Sync timeout in minutes (default: 8 hours)
BLOCK_PROCESSING_DURATION=1320    # Block processing verification in seconds (default: 22 minutes)
BLOCK_STALL_TIMEOUT=600           # Fail if no new block for this many seconds (default: 10 minutes)
NODE_UNRESPONSIVE_TIMEOUT=300     # Fail if node unresponsive for this many seconds (default: 5 minutes)
CHECK_INTERVAL=10                 # How often to check node status in seconds
STATUS_PRINT_INTERVAL=30          # How often to print status in seconds
```

The `MULTISYNC_NETWORKS` variable controls which networks to sync (default: `hoodi,sepolia,mainnet`):

```bash
# Sync only hoodi and sepolia
make multisync-loop MULTISYNC_NETWORKS=hoodi,sepolia
```

### Monitoring Behavior

The `docker_monitor.py` script manages the sync lifecycle:

1. **Waiting**: Node container starting up
2. **Syncing**: Snapsync in progress
3. **Block Processing**: Sync complete, verifying block processing
4. **Success**: Network synced and processing blocks
5. **Failed**: Timeout, stall, or error detected

The monitor checks for:
- Sync timeout (default 8 hours, configurable via `SYNC_TIMEOUT`)
- Block processing stall (default 10 minutes without new blocks, configurable via `BLOCK_STALL_TIMEOUT`)
- Node unresponsiveness (default 5 minutes, configurable via `NODE_UNRESPONSIVE_TIMEOUT`)

### Logs and History

Logs are saved to `tooling/sync/multisync_logs/`:

```
multisync_logs/
├── run_history.log          # Append-only history of all runs
└── run_YYYYMMDD_HHMMSS/     # Per-run folder
    ├── summary.txt          # Run summary
    ├── ethrex-hoodi.log     # Ethrex logs per network
    ├── consensus-hoodi.log  # Lighthouse logs per network
    └── ...
```

### Commands

**Starting and Stopping:**

- `make multisync-up` starts all networks via Docker Compose.
- `make multisync-down` stops and removes containers (preserves volumes).
- `make multisync-clean` stops containers and removes volumes (full reset).
- `make multisync-restart` restarts the cycle (clean volumes + start fresh).

**Monitoring:**

- `make multisync-loop` runs continuous sync cycles (recommended for servers). On success, restarts and syncs again. On failure, stops for debugging.
- `make multisync-run` runs a single sync cycle and exits on completion.
- `make multisync-monitor` monitors already-running containers (one-shot).

**Logs:**

- `make multisync-logs` tails logs from all networks.
- `make multisync-logs-hoodi` tails logs for a specific network.
- `make multisync-logs-ethrex-hoodi` tails only ethrex logs for a network.
- `make multisync-logs-consensus-hoodi` tails only consensus logs for a network.
- `make multisync-history` views the run history log.
- `make multisync-list-logs` lists all saved run logs.

### Slack Notifications

When configured, notifications are sent:
- On **success**: All networks synced and processing blocks
- On **failure**: Any network failed (timeout, stall, or error)

Notifications include:
- Run ID and count
- Host, branch, and commit info
- Per-network status with sync time and blocks processed
- Link to the commit on GitHub
