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

This feature runs EL+CL pairs in parallel via Docker Compose, checks both clients of every pair, sends Slack notifications, and keeps a history log of runs. `multisync.toml` sets which pairs run and on which networks.

### Overview

Each cycle:
- Generates `docker-compose.multisync.json` from `multisync.toml` and starts the selected pairs
- Monitors each EL's snapsync with a configurable timeout (default 8 hours), then verifies block processing (default 22 minutes)
- Checks each CL over the standard Beacon API for the whole cycle (see [CL checks](#cl-checks))
- Sends Slack notifications on success/failure and appends to a history log
- On success: removes containers and volumes, and begins a new cycle
- On failure: keeps containers running for debugging

Errors start with `EL:` or `CL:`, so a failure names the client that broke.

### Configuring pairs

`multisync.toml` lists one `[[pair]]` per EL+CL pair:

```toml
[[pair]]
network = "hoodi"         # mainnet | sepolia | hoodi | plataberget
el = "ethrex"             # ethrex | geth | nethermind
cl = "lighthouse"         # lighthouse | prysm | ethlambda

[[pair]]
name = "hoodi-ethlambda"  # required when a network repeats
network = "hoodi"
el = "ethrex"
cl = "ethlambda"
```

Optional pair keys:

| Key | Meaning |
|-----|---------|
| `name` | Pair name, default the network. Containers are `{el}-{name}` and `{cl}-{name}` |
| `enabled` | `false`: the pair runs only when named in `MULTISYNC_NETWORKS` |
| `sync_timeout` | Minutes; overrides `SYNC_TIMEOUT` |
| `block_processing_duration` | Seconds; overrides `BLOCK_PROCESSING_DURATION`. At least two epochs |
| `el_rpc_port`, `cl_api_port` | Host ports; default `8545 + i` and `5052 + i` for the pair's position `i` in the file |
| `el_metrics_port`, `cl_metrics_port` | Host ports; metrics are published only when set |

Per-client overrides apply to every pair using the client, on every network:

```toml
[clients.ethlambda]
image = "ghcr.io/lambdaclass/ethlambda:beacon"
pull_policy = "always"
extra_args = ["--custody-group-count", "8"]
env = { RUST_LOG = "info" }
```

The committed config reproduces the original matrix: ethrex+Lighthouse on hoodi, sepolia and mainnet (enabled), plus `hoodi-2` and `plataberget` (disabled). Its host ports are pinned, so mainnet's EL metrics stay on 3701 for the central Prometheus.

Pick pairs for a run by name, or point at another config:

```bash
make multisync-loop MULTISYNC_NETWORKS=hoodi,plataberget   # named pairs run even when disabled
make multisync-loop MULTISYNC_CONFIG=my-matrix.toml
```

The config is read again at every restart, so edits apply from the next cycle. Supporting another client takes one class in `multisync/clients.py`.

Host ports with the committed config:

| Pair | EL RPC | EL metrics | CL Beacon API |
|------|--------|------------|---------------|
| hoodi | 8545 | 3702 | 5052 |
| sepolia | 8546 | 3703 | 5053 |
| mainnet | 8547 | 3701 | 5054 |
| hoodi-2 | 8548 | | 5055 |
| plataberget | 8549 | | 5056 |

#### Platåberget

[Platåberget](https://plataberget.dev/) is the public Glamsterdam testnet, run on the glamsterdam-devnet-8 spec. Clients with no built-in name for it (geth, Lighthouse, Prysm) start from files that a `setup-config-plataberget` service downloads once into a shared volume from [ethpandaops/glamsterdam-devnets](https://github.com/ethpandaops/glamsterdam-devnets/tree/master/network-configs/devnet-8/metadata): `config.yaml`, `genesis.ssz`, `genesis.json`, the bootnodes and the deposit metadata (fetched again after `multisync-clean`). ethrex, Nethermind and ethlambda know it by name. It is not in the default set; opt in explicitly:

```bash
make multisync-loop MULTISYNC_NETWORKS=plataberget
```

### CL checks

Every pair's CL is checked over the standard Beacon API, so the same checks judge every CL:

| Check | When | Fails when |
|-------|------|------------|
| Startup | until the first answer | no answer within `CL_STARTUP_TIMEOUT`, or the CL container exits before answering |
| Unresponsive | after the first answer | no answer for `NODE_UNRESPONSIVE_TIMEOUT` |
| Head stall | whole cycle | `head_slot` unchanged for `CL_HEAD_STALL_TIMEOUT` |
| Provider match | whole cycle, every `CL_PROVIDER_CHECK_INTERVAL` | the CL's finalized root differs from the checkpoint provider's at the same epoch |
| Healthy at end | end of block processing | `is_syncing`, `is_optimistic` or `el_offline` is true; the finalized epoch did not advance during the window; or no provider match was seen in it. The flags are re-read for up to three slots, since `is_optimistic` can flip for a slot |
| EL/CL hash | end of block processing | the EL's `finalized` block hash differs from the one the CL's finalized block names (the payload hash, or under Gloas the bid's parent hash). Re-read for up to three slots; fork choice's finalized block leads the head state's after an epoch boundary, so either one is accepted |

The CL Beacon API is published on 127.0.0.1 only. Pinned client images are pulled only when missing; ethlambda's moving `beacon` tag and ethrex always pull.

The EL fails the pair too when its RPC never answers within `EL_STARTUP_TIMEOUT` (the error adds the container's exit code if it exited). An EL counts as synced only once `eth_syncing` is false at a block above 0. The EL and CL windows start together, when the CL starts following.

The checkpoint provider is each network's checkpoint-sync URL. A provider outage alone never fails a pair; only "no match seen during the window" does, and its error says how many provider polls failed.

### Upgrading from the hand-written compose file

1. The first restart under this version removes the old stack's containers and its `ethrex-*` volumes by itself, since they share the compose project `sync`. The old CL volumes are never declared in the new file, so remove them once by hand. Check the exact names first with `docker volume ls --filter name=sync_consensus`.

```bash
docker volume rm sync_consensus-hoodi sync_consensus-sepolia sync_consensus-mainnet sync_consensus-hoodi-2 sync_consensus-plataberget
```

2. A monitor running `--auto-update` on the old code stops at its first pull after this change lands, because it still reads the deleted YAML. Restart it by hand with `make multisync-loop-auto`.

After a pull that moves HEAD, the monitor now restarts itself, so new code and config schema apply without a manual restart.

### Auto-Update Mode with State Trie Validation

The `multisync-loop-auto` target provides continuous integration testing by:
1. **Pulling latest code** from a configured branch before each run
2. **Building the ethrex Docker image** with a configurable Cargo profile; pairs whose EL is ethrex run it, other clients keep their images
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

**Run count persistence:** The run count is persisted across restarts by reading from the history log. If run #5 fails and you restart, the next run will be #6.

### Requirements

- Docker and Docker Compose
- Python 3 with the `requests` library (`pip install requests`); on Python below 3.11 also `tomli` (`pip install tomli`)
- (Optional) Slack webhook URLs for notifications

### Quick Start

```bash
# Start a continuous monitoring loop (recommended for servers)
make multisync-loop

# Or run a single sync cycle
make multisync-run
```

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

# CL checks (optional - values shown are defaults, in seconds)
EL_STARTUP_TIMEOUT=900            # Fail if the EL's RPC never answers within this
CL_STARTUP_TIMEOUT=900            # Fail if the CL's Beacon API never answers within this
CL_HEAD_STALL_TIMEOUT=600         # Fail if the CL head does not move for this long
CL_PROVIDER_CHECK_INTERVAL=60     # How often to compare finality with the checkpoint provider
```

`sync_timeout` and `block_processing_duration` in `multisync.toml` override the first two per pair.

### Monitoring Behavior

The `docker_monitor.py` script manages each pair's lifecycle. The EL moves through:

1. **Waiting**: Node container starting up
2. **Syncing**: Snapsync in progress
3. **Block Processing**: Sync complete, verifying block processing
4. **Success**: Synced, processing blocks, and every CL check passed
5. **Failed**: Timeout, stall, error, or a failed CL check

The CL is checked alongside, from startup to the end of block processing (see [CL checks](#cl-checks)); a CL failure fails the pair at once.

The monitor checks for:
- Sync timeout (default 8 hours, configurable via `SYNC_TIMEOUT`)
- Block processing stall (default 10 minutes without new blocks, configurable via `BLOCK_STALL_TIMEOUT`)
- Node unresponsiveness (default 5 minutes, configurable via `NODE_UNRESPONSIVE_TIMEOUT`)
- CL startup, liveness, head progress and finality (see [CL checks](#cl-checks))

ethrex-only features (`admin_*` diagnostics, watched phases, phase timings, trie validation) run only for pairs whose EL is ethrex.

### Logs and History

Logs are saved to `tooling/sync/multisync_logs/`:

```
multisync_logs/
├── run_history.log            # Append-only history of all runs
└── run_YYYYMMDD_HHMMSS/       # Per-run folder
    ├── summary.txt            # Run summary
    ├── ethrex-hoodi.log       # EL logs per pair ({el}-{pair})
    ├── lighthouse-hoodi.log   # CL logs per pair ({cl}-{pair})
    └── ...
```

### Commands

**Starting and Stopping:**

- `make multisync-compose` generates `docker-compose.multisync.json` from the config.
- `make multisync-up` starts the selected pairs via Docker Compose.
- `make multisync-down` stops and removes containers (preserves volumes).
- `make multisync-clean` stops containers and removes volumes (full reset).
- `make multisync-restart` restarts the cycle (clean volumes + start fresh).

**Monitoring:**

- `make multisync-loop` runs continuous sync cycles (recommended for servers). On success, restarts and syncs again. On failure, stops for debugging.
- `make multisync-run` runs a single sync cycle and exits on completion.
- `make multisync-monitor` monitors already-running containers (one-shot).

**Logs:**

- `make multisync-logs` tails logs from all pairs.
- `make multisync-logs-hoodi` tails a pair's EL and CL logs.
- `make multisync-logs-el-hoodi` (or `multisync-logs-ethrex-hoodi`) tails only the pair's EL.
- `make multisync-logs-cl-hoodi` (or `multisync-logs-consensus-hoodi`) tails only the pair's CL.
- `make multisync-history` views the run history log.
- `make multisync-list-logs` lists all saved run logs.

**Tests:**

- `make multisync-test` runs the tooling's unit tests (no Docker or network needed).

### Slack Notifications

When configured, notifications are sent:
- On **success**: All pairs synced, processing blocks, and passing the CL checks
- On **failure**: Any pair failed (timeout, stall, error, or a CL check)

Notifications include:
- Run ID and count
- Host, branch, and commit info
- Per-pair status (`name: el+cl`) with sync time, blocks processed, and a CL line (head, finalized epochs gained, provider match, EL/CL hash)
- Link to the commit on GitHub
