# L2 TPS Benchmarking Guide

How to measure the transactions per second (TPS) an ethrex L2 settles under a synthetic load. The headline figure is the settled TPS: transactions whose batch has been proved and verified on L1, per second of wall time, so it accounts for the whole settlement path (block production, batch sealing and commitment, proving, L1 verification) and is bounded by its slowest stage. The per-stage figures are measured alongside to show which stage that is. The procedure was written for a single machine with one GPU and two proving backends (SP1 and ZisK), but every step works for any backend the prover supports.

For proving time alone, without the sequencer side, see the [Prover Benchmarking Guide](https://github.com/lambdaclass/ethrex/blob/main/docs/l2/prover-benchmarking.md).

## What is measured

An L2 transaction goes through four stages, and each one has its own throughput:

| Stage | Component | Where the figure comes from | Meaning |
|---|---|---|---|
| Block production | Block producer | `Stored new block …, transaction_count N, tps T` log line and the `transactions_per_second` gauge | Transactions included per second of real block interval. This is the execution ceiling of the sequencer for the workload. |
| Batch sealing | Committer | `Batch B holds blocks F..=L: N transactions in S s, tps T` log line, plus the `batch_tx_count`, `batch_size` and `batch_gas_used` gauges | Sustained rate over a whole batch, from block timestamps. Batches are also capped by what fits in one blob, so this shows the batch size the prover will see. Blocks are capped by the blob or by the L2 block gas limit, whichever binds first. |
| Proving | Prover (`--timed`) | `Proved payload #B … proving_time_ms=M` log line | Proven TPS of a batch is its transaction count divided by its proving time. The aggregate over a run (sum of transactions divided by sum of proving times) is the throughput one GPU can prove. |
| L1 verification (headline) | OnChainProposer | L1 block timestamps of the `verifyBatches` transactions (the sequencer logs each `verify_tx_hash`), plus the per-batch transaction counts | Settled TPS: transactions in the batches verified between the first and the last verification of the run, divided by the wall time between those two L1 timestamps. It is the end-to-end rate the whole pipeline sustains; with one GPU it is set by proving, and it includes the proof coordinator queue and the L1 transactions. |

Settled TPS is computed as a ratio of totals over a window, never as a mean of per-batch ratios: `sum of transactions in the batches verified inside the window / (timestamp of the last verification - timestamp of the first verification)`. The first verification opens the window and its own batches are excluded, since the time they took is not inside it. The window must be long enough for the pipeline to reach steady state, that is, for the prover to be working on a backlog rather than waiting for batches.

The block producer schedules the next block `block_time_ms` after the previous one is stored, so the real block period is the build and store time plus the block time; the per-block TPS is measured against that interval. Block timestamps have second resolution, so the batch figure is a sustained rate, not a per-block one. The committer seals at most one batch per commit interval, whatever the backlog, so the commit interval is a cap on settlement of its own: it is set to one block interval here so that the prover, not the committer, sets the pace.

A run is sized so that the load always exceeds what the sequencer includes: the load test sends every account's transactions as fast as the RPC accepts them, the mempool limit is raised so nothing is evicted, and the test restarts as soon as a round is included. Blocks are then full for the whole measurement window, and the per-block TPS is the sequencer's capacity for that workload rather than the load generator's pace.

## Prerequisites

- A Linux x86-64 host with an NVIDIA GPU. SP1 6.x downloads and runs its own `sp1-gpu-server`, which needs the CUDA 12 runtime; ZisK v1.3.2-alpha needs more than 24 GB of VRAM for GPU proving (the ZisK L2 backend and its guide arrive with the ZisK prover pull request).
- The backend's toolchain installed: `sp1up` for SP1, the ZisK bundle with the STARK and PLONK keys for ZisK.
- `rex` (for the L1 contract calls) and `solc` (for the ZisK verifier) on the `PATH`.
- The ethrex repository checked out at the commit under test.

## Scripts

`scripts/l2_tps/` automates steps 2 to 6 below and produces the summaries this guide quotes:

- `run.sh <backend> <workload>` runs one measurement from a fresh L1 to a run directory (`tps-runs/<timestamp>-<backend>-<workload>/` by default) holding the node, prover and load-test logs, `metrics.txt`, `blocks.csv` (per block: timestamp, transactions, gas, base fee), `gpu.csv` (GPU memory, utilization and power every 30 s), `l1-progress.csv`, `verifications.csv` (L1 timestamp of every verification) and `summary.md`. It also records `commits.csv` (L1 timestamp, gas and blob gas of every commit transaction). It takes the binaries, window, block time, commit interval, load, L2 block gas limit (`L2_GAS_LIMIT`, applied to the bridge and the L2 genesis), extra sequencer flags (`EXTRA_L2_FLAGS`) and a run label (`TAG`) from environment variables documented at its top; the prover's extra environment (for example the CUDA 12 runtime path for SP1, or the glibc tunable ZisK needs) goes in `PROVER_ENV`.
- `collect.py <run_dir>` recomputes a run's summary; `compare.py <run_dir>...` prints one table across runs with the rows shown under Results, including the operator's L1 fee per million transactions priced at `L1_GAS_PRICE_GWEI` (default 10) and `L1_BLOB_GAS_PRICE_GWEI` (default 1) from the environment, plus committed TPS on L1, settlement latency (L1 verification time minus the L2 timestamp of the batch's last block), L1 gas and blob gas per transaction and the committer's cycle time when the run recorded them; `dump_blocks.py` is the per-block RPC dump `run.sh` calls at the end.

```bash
COMPILE_CONTRACTS=true cargo build --release --features l2,l2-sql,gpu,sp1 --bin ethrex
cargo build --release --manifest-path tooling/load_test/Cargo.toml
PROVER_ENV="LD_LIBRARY_PATH=/path/to/cuda-12/lib" scripts/l2_tps/run.sh sp1 eth-transfers
scripts/l2_tps/run.sh sp1 erc20
scripts/l2_tps/compare.py tps-runs/*-sp1-*
```

Run the pairs you want to compare one after the other with the same settings; the backend binary differs per backend, so set `ETHREX_BIN` when switching.

## Step by step

All commands run from the repository root unless noted. The examples use the default test keys from `fixtures/`, which have no value outside the local devnet.

### 1. Build the binaries

Build one `ethrex` binary per backend with GPU proving, and the load test, once:

```bash
COMPILE_CONTRACTS=true cargo build --release --features l2,l2-sql,gpu,sp1 --bin ethrex
cp target/release/ethrex /somewhere/ethrex-sp1
COMPILE_CONTRACTS=true cargo build --release --features l2,l2-sql,gpu,zisk --bin ethrex
cp target/release/ethrex /somewhere/ethrex-zisk
cargo build --release --manifest-path tooling/load_test/Cargo.toml
```

Each build compiles the backend's guest program, so the first one takes a while. Record the commit hash: proving times and guest cycle counts change between commits.

### 2. Start a fresh L1 and deploy the contracts requiring the backend's proofs

Every run starts from an empty L1 so that batch numbers, contract addresses and the timeline are comparable between runs. From `crates/l2`:

```bash
rm -rf dev_ethrex_l1 dev_ethrex_l2 ../../cmd/.env
ethrex-<backend> --network ../../fixtures/genesis/l1.json --http.port 8545 --http.addr 0.0.0.0 \
  --authrpc.port 8551 --dev --datadir dev_ethrex_l1 > l1.log 2>&1 &
```

For ZisK, deploy the verifier from the pinned release first and keep its address (`make deploy-zisk-verifier` prints it). Then deploy the L1 contracts with the flag that requires the backend's proofs, `--sp1 true` or `--zisk true --zisk.verifier-address <address>`:

```bash
ethrex-<backend> l2 deploy \
  --eth-rpc-url http://localhost:8545 --private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
  <verifier flags> \
  --on-chain-proposer-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 --bridge-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
  --bridge-owner-pk 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e --deposit-rich \
  --private-keys-file-path ../../fixtures/keys/private_keys_l1.txt \
  --genesis-l1-path ../../fixtures/genesis/l1.json --genesis-l2-path ../../fixtures/genesis/l2.json
```

The deployer writes the contract addresses to `cmd/.env`.

### 3. Start the L2 sequencer with a mempool large enough for the load

```bash
set -a; . ../../cmd/.env; set +a
ETHREX_NO_MONITOR=true ethrex-<backend> l2 \
  --watcher.block-delay 0 --network ../../fixtures/genesis/l2.json \
  --http.port 1729 --http.addr 0.0.0.0 --metrics --metrics.port 3702 --datadir dev_ethrex_l2 \
  --mempool.maxsize 1000000 \
  --l1.bridge-address "$ETHREX_WATCHER_BRIDGE_ADDRESS" \
  --l1.on-chain-proposer-address "$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS" \
  --eth.rpc-url http://localhost:8545 --osaka-activation-time 1761677592 \
  --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
  --block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
  --block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
  --block-producer.operator-fee-per-gas 1000000000 \
  --block-producer.block-time 5000 --committer.commit-time 5000 \
  --committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
  --proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
  --proof-coordinator.addr 127.0.0.1 > l2.log 2>&1 &
```

`--mempool.maxsize` matters: the default of 10,000 transactions is below one load-test round, and an eviction drops the lowest nonces of an account, which leaves all of its later transactions stuck. The commit interval equals the block interval so that every block is batched as soon as it exists; a longer interval caps settlement at one batch per interval regardless of how fast the prover is. Keep the block time and commit time fixed across the runs you want to compare; they define the block gas budget per second and the batch size.

### 4. Start the load, then the prover

Start the load test before the prover so that the first proved batches already contain transactions. The mempool admits a transaction only while the sender's cumulative pending cost (gas limit times fee cap, over its pending transactions) fits its balance, so the fee cap the load test puts on its transactions bounds how many each account can have in flight; the tool uses one million gwei, which keeps hundreds of transactions per account admissible against the devnet balances while leaving room for the base fee to climb. `-t eth-transfers` sends plain value transfers; `-t erc20` deploys a token, mints to every account and sends token transfers. `LOAD_TEST_ENDLESS=true` starts a new round as soon as the previous one is fully included:

```bash
LOAD_TEST_RPC_URL=http://localhost:1729 LOAD_TEST_TX_AMOUNT=300 LOAD_TEST_ENDLESS=true \
  ./tooling/target/release/load_test -k ./fixtures/keys/private_keys.txt -t eth-transfers > loadtest.log 2>&1 &
```

Then the prover, with timing enabled. From `crates/l2`:

```bash
ethrex-<backend> l2 prover --backend <backend> --proof-coordinators tcp://127.0.0.1:3900 --timed > prover.log 2>&1 &
```

ZisK on a glibc 2.41 host needs `GLIBC_TUNABLES=glibc.rtld.execstack=2` in the prover's environment; SP1 needs `libcudart.so.12` on the library path if the host ships a newer CUDA.

### 5. Let it run, sampling the L1

Leave the run for a fixed window (15 minutes gives about a dozen batches with the settings above) and sample the OnChainProposer every 30 seconds:

```bash
echo "utc,committed,verified" > l1-progress.csv
while true; do
  V=$(rex call "$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS" "lastVerifiedBatch()" --rpc-url http://localhost:8545 | tail -1)
  C=$(rex call "$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS" "lastCommittedBatch()" --rpc-url http://localhost:8545 | tail -1)
  echo "$(date -u +%FT%TZ),$((C)),$((V))" >> l1-progress.csv
  sleep 30
done
```

At the end, dump the metrics before stopping anything, since the batch gauges live in the sequencer process:

```bash
curl -s localhost:3702/metrics > metrics.txt
```

### 6. Read the results

The sequencer log carries the first two stages:

```
2026-10-16T02:10:05.120Z  INFO Stored new block 9c…, transaction_count 1412, tps 268.3
2026-10-16T02:11:02.441Z  INFO Batch 7 holds blocks 61..=72: 16944 transactions in 63 s, tps 269.0
```

The prover log carries the third:

```
2026-10-16T02:12:40.003Z  INFO Proved payload #7 in 41.20s id=7 proving_time_s=41 proving_time_ms=41203
```

Proven TPS for batch 7 is `16944 / 41.2 = 411`. The settled TPS comes from the L1: for every `Sent batch verification transaction … verify_tx_hash=…` line in the sequencer log, fetch the receipt and its block's timestamp; then divide the transactions of the batches verified after the first verification by the seconds between the first and the last verification timestamps. The 30-second `lastVerifiedBatch()` samples give the same figure at coarser resolution and show whether the verified count was still climbing when the run ended.

Discard the first blocks (the mempool is still filling) and the last ones (the load test was stopped) when averaging the per-block TPS. Report the aggregate figures (transactions over time) rather than the mean of per-batch ratios, which weights small batches as much as full ones.

### 7. Repeat per backend and workload

Stop the prover, the load test and both nodes, wipe the datadirs and go back to step 2 for the next combination. Everything else (binaries, block time, commit time, mempool size, transactions per account, window length) stays the same, so the only variable between two runs is the backend or the workload. One prover at a time: two provers sharing a GPU would measure contention, not throughput.

## Results

Measured on 2026-10-16 on one machine: AMD Ryzen 9 9950X3D (16 cores), 60 GB RAM, NVIDIA RTX 5090 (32 GB), Debian 13, CUDA 13.0 with the CUDA 12.8 runtime for SP1's GPU server, SP1 6.9.0 (Groth16 proofs), ZisK v1.3.2-alpha (PLONK proofs), ethrex main at `4afa894e6` with the ZisK L2 backend (pull request #7423) and the sequencer, committer and load-test fixes from this guide's pull request applied. Settings for every run: 5-second blocks, 5-second commit interval, mempool limit 1,000,000, 170 accounts sending 300 transactions per round without pause, 15-minute window, one prover on the GPU, L1 dev node on the same host.

| Metric | ZisK, ETH transfers | ZisK, ERC20 transfers | SP1, ETH transfers | SP1, ERC20 transfers |
|---|---|---|---|---|
| Settled TPS on L1 (headline) | 49.7 | 25.9 | 19.5 | 11.3 |
| Settlement window (s) | 864 | 854 | 832 | 821 |
| Committed TPS on L1 | 108.4 | 55.7 | 96.4 | 49.6 |
| Sequencer TPS (steady window) | 204.9 | 105.6 | 198.9 | 105.5 |
| Transactions per block | 1,055 | 533 | 1,061 | 533 |
| Gas per block | 22,139,741 | 27,688,118 | 22,287,831 | 27,688,114 |
| Gas per transaction | 21,000 | 52,114 | 21,000 | 52,114 |
| Batches proved in the window (the deposit batch excluded) | 40 | 43 | 15 | 18 |
| Transactions per batch | 1,075 | 528 | 1,079 | 516 |
| Proving time per batch (s) | 16.5 | 15.3 | 50.2 | 40.5 |
| Proven TPS (transactions / proving time) | 65.0 | 34.4 | 21.5 | 12.7 |
| Proving seconds per Mgas | 0.73 | 0.56 | 2.21 | 1.51 |
| L1 commit gas per batch | 185,482 | 185,479 | 185,478 | 185,479 |
| L1 blob gas per batch | 131,072 | 131,072 | 131,072 | 131,072 |
| L1 verify gas per batch | 413,486 | 413,688 | 349,337 | 349,345 |
| L1 execution gas per transaction (commit + verify) | 558 | 1,136 | 496 | 1,037 |
| L1 blob gas per transaction | 122 | 248 | 121 | 254 |
| L1 fee per million transactions (ETH, at 10 gwei gas and 1 gwei blob gas) | 5.70 | 11.61 | 5.08 | 10.62 |
| Prover GPU time per transaction (ms, 1 / settled TPS) | 20.1 | 38.6 | 51.4 | 88.4 |
| GPU memory, max (MiB) | 30,330 | 29,948 | 26,038 | 26,102 |
| GPU utilization, mean (%) | 61 | 51 | 34 | 37 |
| GPU power, mean (W) | 398 | 433 | 230 | 211 |

GPU sampling was added after the first ZisK run had started, so ZisK with ETH transfers was measured a second time with every sample in place; the GPU rows of that column come from the repeat. The repeat reproduced the first run within noise: settled TPS 49.7 in both, sequencer 205.2 against 204.9 TPS, proving 16.5 s per batch in both, the same 40 batches proved in the window.

How to read it:

- Every batch is one full block, but the two workloads fill it differently. ETH-transfer blocks are full by bytes: about 1,060 transfers of 117 bytes fill the blob that publishes the batch, at 22.3M gas. ERC20-transfer blocks are full by gas: the deployer stores a 30M L2 block gas limit by default (`--l2-gas-limit`) and the load test gives every transaction a 2.1M gas limit, so the builder stops at 27.9M gas, 536 transfers and about 99 KB, which leaves no room for a second block in the batch's blob. The sequencer figures are therefore the blob capacity per block time for ETH transfers and the gas limit per block time for ERC20 transfers.
- The settled TPS is below the proven TPS by the time the prover spends between proofs (fetching the next batch input, the coordinator round trip, submitting the proof): the GPU sits idle roughly a quarter to a half of the time with one prover per GPU, which the GPU utilization row shows. Running two prover processes per GPU, or pipelining input fetch with proving, would recover most of it.
- Behind the prover sits the committer, which also falls short of the sequencer. After each commit it waits a full commit interval before the next one, and a commit cycle (L1 reads, batch preparation and witness, sending the transaction and polling its receipt every 2 s) takes about 4 to 6 s, so with one block per batch it publishes one batch every 9 to 11 s at 5-second intervals: 108 ETH transfers per second against the sequencer's 205. A faster prover alone would therefore settle at most about 108 TPS of ETH transfers with these settings.
- Proving seconds per Mgas is the figure to use for capacity planning; it depends on the workload (ERC20 transfers cost more gas per transaction but prove faster per gas than plain transfers, whose cost is dominated by signature recovery and state writes). Neither backend proves a block within the 5-second block time on this GPU: a full ETH-transfer block takes 3.3 block times on ZisK and 10 on SP1, so settlement is prover-bound and throughput parity would take about four ZisK provers or ten SP1 provers, each on its own GPU; a proof landing before the next block would need one batch split across GPUs, which the ethrex prover does not do.
- L1 cost per batch is independent of the workload: one blob, a commit transaction of about 185k gas, and a verification of about 413k gas for a ZisK PLONK proof or about 349k gas for an SP1 Groth16 proof.
- The operator's L1 cost per transaction is the batch's commit and verification execution gas, plus its blob gas, divided by the transactions in the batch. Both are fixed per batch, so the cost per transaction falls as batches hold more transactions: an ERC20 batch holds half as many transfers as an ETH one, so it costs twice as much per transfer. The fee row prices it at 10 gwei per execution gas and 1 gwei per blob gas, which are example prices rather than market data; the cost scales linearly with both, and the scripts take other prices (see Scripts). Execution gas dominates at these prices, so a cheaper proof to verify (SP1's Groth16 against ZisK's PLONK) shows up directly. The prover's cost per transaction is its GPU time, 1 / settled TPS when the GPU proves nonstop; multiply by a GPU-hour price to compare it with the L1 fee.

### Sensitivity: 1-second blocks

ETH transfers with the block time and the commit interval at 1 second instead of 5 (12-minute windows, everything else equal), for both backends:

| Metric | ZisK, 5 s blocks | ZisK, 1 s blocks | SP1, 5 s blocks | SP1, 1 s blocks |
|---|---|---|---|---|
| Settled TPS on L1 | 49.7 | 50.0 | 19.5 | 19.5 |
| Committed TPS on L1 | 108.4 | 177.5 | 96.4 | 152.8 |
| Sequencer TPS (steady window) | 204.9 | 989.9 | 198.9 | 968.0 |
| Transactions per block | 1,055 | 1,062 | 1,061 | 1,062 |
| Transactions per batch | 1,075 | 1,076 | 1,079 | 1,079 |
| Proving time per batch (s) | 16.5 | 16.5 | 50.2 | 50.7 |
| Proven TPS | 65.0 | 65.1 | 21.5 | 21.3 |
| GPU utilization, mean (%) | 61 | 63 | 34 | 35 |
| L1 execution gas per transaction (commit + verify) | 558 | 557 | 496 | 496 |
| L1 blob gas per transaction | 122 | 122 | 121 | 121 |
| L1 fee per million transactions (ETH, at 10 gwei gas and 1 gwei blob gas) | 5.70 | 5.69 | 5.08 | 5.08 |
| Prover GPU time per transaction (ms, 1 / settled TPS) | 20.1 | 20.0 | 51.4 | 51.4 |

The block builder fills a blob-sized block in well under a second, so block production scales with the block time (about 1,000 ETH transfers per second at 1-second blocks) while the settled figure does not move for either backend: it is set by the prover, which still takes 16.5 s (ZisK) or 50 s (SP1) per blob-sized batch. The runs also show the next cap behind the prover: the committer waits a full commit interval after each commit and its own cycle takes about 6 seconds here, so at 1-second intervals it publishes about 180 ETH transfers per second (177.5 and 152.8 TPS committed), and at 5-second intervals about 100, however many provers there were. Raising sequencer throughput past that needs fewer bytes per transaction in the blob (state diffs or compression) or several blobs per batch, and a committer that pipelines its commit transactions.

## Design notes

- **Per-block TPS uses the measured interval, not the configured block time.** The producer schedules the next block after the previous one is stored, so under load the period is the block time plus the build time, and dividing by the configured block time overstates throughput.
- **Batch TPS uses block timestamps**, from the parent of the first block to the last block of the batch, which covers every block interval the batch spans. It is a sustained figure with one-second resolution.
- **Settled TPS is the headline** because it is the only figure that accounts for the whole settlement path; the others locate the bottleneck. Proven TPS divides by proving time only: witness generation, queueing in the proof coordinator and the L1 transactions are not in it, but they are in the settled TPS.
- **The commit interval is a settlement cap of its own.** The committer seals at most one batch per interval; the default 60 s would bound settlement at one batch per minute whatever the prover does, so the runs use one block interval.
- **The blob caps the block, and so the sequencer.** The block builder stops adding transactions when the block's encoding would exceed `SAFE_BYTES_PER_BLOB`, because a batch is published as one blob holding its blocks in full, and the committer stops adding blocks to a batch for the same reason. A block of plain ETH transfers tops out around 1,080 transactions, so with 5-second blocks the sequencer cannot settle more than about 215 TPS of them whatever the prover does. ERC20 transfers hit the L2 block gas limit first (30M by default, set at deployment), at 536 transfers and about 99 KB; raising that limit lets them fill the blob too, at about 680 transfers. Shorter blocks raise both caps, bigger blobs or compressed batch data would raise the blob one.
- **Two defects found by the first runs are fixed alongside this guide.** The block builder's blob-space check ignored the framing the committer adds around a block in the batch blob (the 8-byte block count, the RLP list prefixes that grow as the block fills, the header's `gas_used`), so under sustained load a block landed a few bytes over the blob within the first twenty-odd blocks and the committer, which cannot split a block, stopped committing for good; the builder now reserves that margin. The committer also counted a block's transactions and gas toward a batch before checking whether the block fit, so every batch closed by a block that did not fit reported twice its transactions in the `batch_tx_count` gauge; the counters now move with the block's acceptance. Older figures derived from that gauge should be treated as suspect.
- **Why a fresh L1 per run.** Contract addresses, batch numbers and the L1 block timeline are then comparable across runs, and no batch from a previous run is left for the new prover.

## References

- [Prover Benchmarking Guide](https://github.com/lambdaclass/ethrex/blob/main/docs/l2/prover-benchmarking.md) and the [agent workflow](https://github.com/lambdaclass/ethrex/blob/main/docs/workflows/prover_benchmarking.md) it comes from, which this guide extends with the sequencer and L1 stages.
- [Load test tool](https://github.com/lambdaclass/ethrex/tree/main/tooling/load_test) (`tooling/load_test/README.md`).
- [`scripts/bench_metrics.sh`](https://github.com/lambdaclass/ethrex/blob/main/scripts/bench_metrics.sh), which tabulates proving times with the batch gauges.
- [SP1 prover guide](./deployment/prover/sp1.md) and [SP1 upgrade notes](../developers/l2/sp1-upgrade-test.md) for the backend prerequisites; the ZisK guide ships with the ZisK L2 backend pull request.
