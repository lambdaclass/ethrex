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

## Full-sync throughput regression watch

Tracks execution throughput on real mainnet/testnet blocks over time, to catch performance
regressions that land on `main`. Complements the existing coverage rather than duplicating
it: the per-PR CI benchmark runs a synthetic dense-ERC20 import, and multisync validates
*snap* sync completion — neither watches execution throughput drift. Design: issue #7111.

### How it works

A node is kept permanently a fixed distance behind the tip. Each cycle both *consumes* a
saved database and *produces* the next one, so the whole thing is a loop that bootstrap
seeds once and never needs re-anchoring:

```
        ┌────────────────────────────────────────────────┐
        │                                                │
        ▼                                                │
   base.0  (a stopped node's datadir, at block B)        │
        │                                                │
        ├── restore ─► MEASURE  B → B+M ──► throughput; datadir discarded
        │                                                │
        └── restore ─► ADVANCE  B → T−GAP ─► new base.0 ─┘
```

One cycle, per network, once a day:

1. **Gate.** Read B from the base's `bench-base.json` and estimate the tip T. If
   `T − B < M` the whole window has not been produced yet, so skip and try later.
2. **Measure leg.** Restore `base.0` into the datadir — a *copy*; no node ever opens the
   base itself. Re-sync the beacon node ahead of B, drop the page cache, start the node,
   let it full-sync `B → B+M`, stop it gracefully, and read the per-batch throughput out
   of its log. **The resulting database is thrown away**; only the number is kept.
3. **Advance leg.** Restore `base.0` again, back to B. Sync `B → T−GAP`, stop, then
   health-check the result by reopening it. Only if it comes back up with state available
   does it rotate `base.0→base.1→…` and become the new `base.0`.
4. **Report** the measurement.

```
     B (7d behind)        B+M (4d behind)                 T (tip)
     │                        │                            │
     ├──── measure: 21,600 ──►│  discarded
     ├─ advance: 7,200 ─►│
                  new base (6d behind)
```

**Why two runs from the same point.** The measurement window `M` must never change or the
history stops being comparable, while the base has to advance at exactly chain rate — one
day per day — or it either catches the tip or falls away forever. Those are two different
distances, so they need two different runs. Promoting the measurement leg's database
instead would advance the base by `M` every day, closing the gap within days and breaking
the mechanism. The measurement is therefore deliberately sterile: it starts from the base,
yields a number, and its state is destroyed.

The overlap is the payoff. Consecutive three-day windows measured from a base that moved
only one day share **two-thirds of their blocks**, so day-to-day workload variation stays
small and a real regression shows up as a step in the series rather than as noise.

The advance targets `T − GAP` rather than a fixed block count, so it self-calibrates to
each network's real block production and absorbs missed slots.

Rough cost per network per day on mainnet: two restores at ~6 min each, ~50 min measuring,
~17 min advancing.

### Usage

```bash
make fullsync-bench-bootstrap                              # first base per network (once)
make fullsync-bench-once                                   # one cycle, then exit
make fullsync-bench-watch                                  # continuous
make fullsync-bench-watch FULLSYNC_BENCH_NETWORKS=mainnet,sepolia,hoodi
make fullsync-bench-test                                   # unit-test metric parsing
```

Networks run **serially** on purpose: two legs at once contend for CPU and disk and both
results are junk. An flock enforces this; bootstraps share it, cycles take it exclusively,
and the manual A/B tool (#7112) takes it exclusively too.

Reporting goes to `SLACK_WEBHOOK_URL_SUCCESS`, or `SLACK_WEBHOOK_URL_FAILED` for a round
where a cycle could not produce a valid measurement — a crashed node, an unclean exit, a
leg that never reached its target. A merely *low* number is not a failure while the watch
is observe-only: there is no baseline to judge it against yet. Both are read from
`tooling/sync/.env` (same format as multisync) or the environment, which wins.

**Each leg re-checkpoint-syncs the beacon node to the tip first, and this is load-bearing.**
The execution node only does bulk 1024-block batch sync when fork choice hands it a head
far ahead of itself. A beacon node stopped alongside it knows nothing newer than the base,
so the pair crawl forward together importing ~30 blocks at a time — which measures
incremental block import rather than full sync, and emits no throughput metric at all. A
leg whose beacon node cannot reach the tip is reported `cl_not_synced` rather than
recording a number that means something else.

### Bootstrap

`make fullsync-bench-bootstrap` snap-syncs each network to the tip, stops it gracefully and
records it as `base.0` plus a `bench-base.json` holding its block number. Snap is used for
this initial fill only — full-syncing from genesis would take weeks; the measurement legs
themselves always run full, since that is the thing being measured.

The node is then simply left stopped. The gap opens on its own at one day per day as the
chain moves on, so nothing needs anchoring by hand. Cycles skip themselves with a log line
until `head − base ≥ M`; with the default `M` of 3 days that means roughly a three-day wait
after bootstrap before the first real measurement (six to reach the steady-state `GAP`).

Run **observe-only for 2–3 weeks** after that before wiring up alerting: the real
day-to-day σ is unknown, and thresholds should come from measured data rather than a guess.
Alerting and step detection (trailing median + persistence, not day-vs-day) land later.

### Rehearsing a cycle

Waiting `M` days to discover that a log format moved or a webhook is wrong is a poor
feedback loop, so `make fullsync-bench-smoke` runs one complete cycle immediately against
a small window:

```bash
make fullsync-bench-smoke                          # hoodi, 2048-block window
make fullsync-bench-smoke FULLSYNC_SMOKE_NET=sepolia
```

It exercises the real path end to end — restore, a genuine full-sync leg, graceful stop,
metric extraction from live logs, the result JSON, base rotation and the Slack post — on a
`cp -al` hardlink copy of a real base, so it costs almost no disk and cannot damage the
live series. Stop the watch first; it holds the lock, and the smoke run will refuse.

Keep the window at or above ~2048 blocks. Throughput is read from the per-batch metric
line, which ethrex emits every 1024 blocks, so a smaller window can finish having logged
no batch at all and be reported `no_batches`.

`--measure-blocks` and `--gap-blocks` exist for this and nothing else. The runner refuses
to accept them alongside the default results directory, because folding a 2k-block sample
into a series of 21.6k-block ones would drag the trailing median that the comparison rests
on.

### What is being measured

The compose runs `ghcr.io/lambdaclass/ethrex:main` with `pull_policy: always`, so each node
start re-pulls the latest published build: the series tracks `main` over time rather than a
pinned binary. That is the intent, but it means a sample is only interpretable alongside the
build that produced it, which is why every result records `measured.revision` (the ethrex
commit from the image's `org.opencontainers.image.revision` label) separately from
`tooling_commit`.

Point `ETHREX_IMAGE` at a local tag with `ETHREX_PULL_POLICY=never` to measure something
else — that is how the manual A/B tool (#7112) will pin its refs.

### Storage layout

Everything lives under `BENCH_DATA_ROOT` (default `/mnt/raid10/fullsync-bench`):

```
<root>/data/<net>/         bind-mounted into the container as /data
<root>/consensus/<net>/    beacon node database
<root>/state/<net>/base.N  retained base generations (BENCH_KEEP_GENERATIONS, default 2)
<root>/results/<net>/      one JSON + log per leg
```

Node data and bases **must stay on one filesystem**. `snapshot()` hardlinks unchanged SST
files between generations with `rsync --link-dest`, which only works within a filesystem;
across two, rsync silently copies instead and every retained generation costs a full
database rather than a delta. The runner asserts this at startup rather than trusting it.

Bind mounts are used instead of named Docker volumes for the same reason: Docker's
`data-root` is usually on the OS disk, which is both smaller and a different filesystem
from the array holding the bases.

**Sizing.** Measured on mainnet: a base is ~285 G and a full measurement leg grows the live
data dir by a further ~84 G, so peak is ~370 G plus one base per retained generation, plus
~4.5 G for the beacon node. Sepolia is ~217 G / +24 G, hoodi ~49 G / +4 G. All three
networks at two generations land around 1.2 T; mainnet alone around 700 G. Bases also grow
with chain state, so provision headroom rather than exactly.

`BENCH_KEEP_GENERATIONS` (default 2) is the cheapest lever on a small disk — it is pure
rollback insurance and does not affect measurements. Each generation costs its own delta
*plus* the dead SST files its hardlinks keep alive against compaction. 2 is the minimum:
rotation deletes the oldest generation before the replacement snapshot exists, so a single
generation would leave nothing behind if that snapshot failed.

### Metrics

Each cycle writes one JSON per leg under `<results-dir>/<network>/`:

| field | why |
|---|---|
| `throughput_ggas_s` (mean/median/stdev/samples) | headline; the **per-batch mean**, not blocks ÷ wall clock |
| `blocks_per_s_mean` | likely the better primary on light testnet blocks |
| `phase_ms_per_mgas` (`validate`/`exec`/`merkle`/`store`) | catches phase-localised regressions invisible end-to-end |
| `state_regen_seconds` | restart cost; sensitive to commit cadence |
| `wall_seconds`, `status`, `reached_block`, `host` | validity |
| `measured` (image, image_id, revision) | **which ethrex build produced this sample** |
| `tooling_commit` | commit of the benchmarking code itself |

Wall clock is deliberately *not* the throughput metric: it includes startup state
regeneration, which is a real signal but a separate one, so it is reported on its own.

### Operational invariants

These are lessons from the manual benchmarking that preceded this tool, not preferences:

- **Graceful stop only** (`docker stop -t 300`). Repeated abrupt stops previously left the
  canonical head ahead of any durably-flushed state, after which the node could not
  regenerate and needed a full re-sync.
- **Stop condition reads `eth_blockNumber`**, never `eth_syncing.currentBlock` — the latter
  goes stale during catch-up and once let a leg run far past its target.
- **Health-gate base rotation**: a new base is promoted only after the node demonstrably
  restarts on it with state available; otherwise the previous generation is kept.
- **Never compare across machines.** The same binary has measured 62 s and 81 s on
  different CI runners; only same-box comparisons mean anything.
- **Do not `--force-recreate` a `consensus-*` service on its own.** That re-runs its
  `setup-jwt-*` dependency, which writes a fresh `jwt.hex`, while the already-running
  execution node keeps the secret it read at startup. Engine API calls then fail with
  `Auth failed`, the node stops being told where the chain tip is, and the sync quietly
  goes nowhere. Restart the matching `ethrex-*` container afterwards.
- **During snap sync `eth_blockNumber` stays at 0** and jumps to the tip only once the
  pivot state has landed. A runner sitting at `0 / <tip>` for hours is normal, not a
  stall; `docker logs ethrex-<net>` is where the real progress is.
