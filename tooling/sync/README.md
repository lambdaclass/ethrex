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
