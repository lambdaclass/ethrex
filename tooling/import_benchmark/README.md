# Import Benchmark

## Why
This tool is used to benchmark the performance of ethrex. We should execute
always the same blocks on the same computer, which is hard to do on a running node.
We also have differences in hardware, number of peers and other load on the computer.
As such we would want to run the same blocks multiple times on the same machine, with
the command import-bench.

## Setup
To run this benchmark, we require:
- A database of ethrex, as we need state to bench real performance of the database, located in ~/.local/share/ethrex_NETWORK_bench/ethrex
- The database should have the snapshots (flatkeyvalue generation) finished. (In mainnet this takes about 8 hours)
- We need to have a chain.rlp file with the files we want to tests, located in ~/.local/share/ethrex_NETWORK_bench/chain.rlp
- It's recommended it has a least a 1000 blocks, and it can be created with the export subcommand in ethrex

The recommended way to have this, is:
- Run an ethrex node until it syncs and generates the snapshots
- Once this is done, shut down the node and copy the db and last block number
- Restart the node until the network has advanced X blocks
- Stop the node and run the `ethrex export --first block_num --last block_num+x ~/.local/share/ethrex_NETWORK_bench/chain.rlp` command

## Run
To makefile includes the following command:

```
run-bench: ## Runs a bench for the current pr.
Parameters
 -BENCH_ID: number for the log file where it will be saved with the format bench-BENCH_ID.log
 -NETWORK: which network to acesss (hoodi, mainnet)
```

## View output
You can view the output with the following command:

`python3 parse_bench.py bench_num_1 bench_num_2`

