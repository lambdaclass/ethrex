# Import Benchmark

## Why

This tool is used to benchmark the performance of **ethrex**.  
We aim to execute the same set of blocks on the same hardware to ensure consistent 
performance comparisons. Doing this on a running node is difficult because of variations 
in hardware, peer count, block content, and system load.

To achieve consistent results, we run the same blocks multiple times on the same machine 
using the `import-bench` subcommand.

## Setup

To run this benchmark, you will need:

- An **ethrex** database containing the blockchain state (required for realistic
 database performance testing), located at:  
  `~/.local/share/ethrex_NETWORK_bench/ethrex`
- The database **must have completed snapshot generation** (`flatkeyvalue` generation).  
  *(On mainnet, this process takes about 8 hours.)*
- A `chain.rlp` file containing the blocks you want to test, located at:  
  `~/.local/share/ethrex_NETWORK_bench/chain.rlp`
- It is recommended that the file contains **at least 1,000 blocks**, 
which can be generated using the `export` subcommand in ethrex.

### Recommended procedure

1. Run an ethrex node until it fully syncs and generates the snapshots.  
2. Shut down the node and copy the database and the last block number.  
3. Restart the node and let it advance by *X* additional blocks.  
4. Stop the node again and run:  
   ```bash
   ethrex export --first <block_num> --last <block_num + X> ~/.local/share/ethrex_NETWORK_bench/chain.rlp
   ```

## Run

The Makefile includes the following command:

```
run-bench: ## Runs a benchmark for the current PR.
```

Parameters:
  - BENCH_ID: Identifier for the log file, saved as bench-BENCH_ID.log
  - NETWORK: Network to access (e.g., hoodi, mainnet)

Example:

`make run-bench BENCH_ID=1 NETWORK=mainnet`

## View Output

You can view and compare benchmark results with:
`python3 parse_bench.py <bench_file_1> <bench_file_2> <...>`
