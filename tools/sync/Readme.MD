# Sync tools guide

The Makefile in this following has some useful targets to run both syncing with chains, and measure the performance when syncing.

## Syncing

There are two targets to sync: sync-levm and sync-revm. Each of them runs a sync with levm and revm, respectively. Optionally, the network to sync with can be set with the variable NETWORK; by default it uses Holesky.

## Syncing logs 

There are two targets for tailing specific parts of the sync logs. tail-syncing-logs tails the logs filtering for the ones that relate to syncing progress; this is useful to tell whether the sync is running. tail-metrics-logs filters for the metrics logs; this is useful to follow the metrics for syncing speed.

## Flamegraph

There are two targets for flamegraphs. flamegraph-main runs a flamegraph on main; flamegraph-branch does it on a branch set with the BRANCH variable. Additionally, the network and evm to use can be set with the NETWORK and EVM variables respectively. The logs for the flamegraph runs are stored in the logs folder under the naming format
`ethrex-[network]-[evm]--flamegraph-[date and time]-[branch]-[logname].svg`. The logname part can be set with the LOGNAME variable.

## DB Backups 
There are two targets to back up the database. backup-db-levm creates a backup for levm, backup-db-revm does it for LEVM. The database is, by default, stored in /home/admin/.local/share/(evm)_(network)_data/ethrex; and the backups are created in a folder called backups inside said folder, in folders with the current date that contain a copy of the database and the logs up to that point. 
