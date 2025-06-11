# Running Ethrex in Aligned Mode

This document explains how to run an Ethrex L2 node in **Aligned mode** and highlights the key differences in component behavior compared to the default mode.

## How to Run
> [!IMPORTANT]  
> For this guide we assumed that there is an L1 running with all Aligned environment set.

### 1. Deploying L1 Contracts

In a console with `ethrex/crates/l2` as the current directory, run the following command:

```bash
cargo run --release --bin ethrex_l2_l1_deployer --manifest-path contracts/Cargo.toml -- \
	--genesis-l1-path <GENESIS_L1_PATH> \
	--genesis-l2-path <GENESIS_L2_PATH> \
	--contracts-path contracts \
	--sp1.verifier-address 0x00000000000000000000000000000000000000aa \
	--pico.verifier-address 0x00000000000000000000000000000000000000aa \
	--risc0.verifier-address 0x00000000000000000000000000000000000000aa \
	--tdx.verifier-address 0x00000000000000000000000000000000000000aa \
    --aligned.aggregator-address <ALIGNED_PROOF_AGGREGATOR_SERVICE_ADDRESS> \
    --bridge-owner <ADDRESS> \
	--on-chain-proposer-owner <ADDRESS> \
	--private-keys-file-path <PRIVATE_KEYS_FILE_PATH> \
	--sequencer-registry-owner <ADDRESS>
```

> [!NOTE]  
> In this step we are initiallizing the `OnChainProposer` contract with the `ALIGNED_PROOF_AGGREGATOR_SERVICE_ADDRESS` and skipping the rest of verifiers.  
> Save the addresses of the deployed proxy contracts, as you will need them to run the L2 node.

### 2. Deposit funds to the `AlignedBatchePaymentService` contract from the proof sender:

```bash
aligned \
--network <NETWORK> \
--private_key <PROOF_SENDER_PRIVATE_KEY> \
--amount <DEPOSIT_AMOUNT>
```
> [!IMPORTANT]  
> Using the [Aligned cli](https://docs.alignedlayer.com/guides/9_aligned_cli)

### 3. Running a node

In a console with `ethrex/crates/l2` as the current directory, run the following command:

```bash
cargo run --release --manifest-path ../../Cargo.toml --bin ethrex --features "l2" -- \
	l2 init \
	--watcher.block-delay <WATCHER_BLOCK_DELAY> \
	--network <L2_GENESIS_FILE_PATH> \
	--http.port <L2_PORT> \
	--http.addr <L2_RPC_ADDRESS> \
	--evm levm \
	--datadir <ethrex_L2_DEV_LIBMDBX> \
	--bridge-address <BRIDGE_ADDRESS> \
	--on-chain-proposer-address <ON_CHAIN_PROPOSER_ADDRESS> \
	--proof-coordinator-listen-ip <PROOF_COORDINATOR_ADDRESS> \
	--aligned \
    --aligned-verifier-interval-ms <ETHREX_ALIGNED_VERIFIER_INTERVAL_MS> \
    --beacon_url <ETHREX_ALIGNED_BEACON_CLIENT_URL> \ 
    --aligned-network <ETHREX_ALIGNED_NETWORK> \
    --fee-estimate <ETHREX_ALIGNED_FEE_ESTIMATE> \
    --aligned-sp1-elf-path <ETHREX_ALIGNED_SP1_ELF_PATH>
```
Aligned params explanation:

- `--aligned`: Enables aligned mode, enforcing all required parameters.
- `ETHREX_ALIGNED_VERIFIER_INTERVAL_MS`: Interval in millisecs, that the `proof_verifier` will sleep between each proof aggregation check.
- `ETHREX_ALIGNED_BEACON_CLIENT_URL`: URL of the beacon client used by the Aligned SDK to verify proof aggregations.
- `ETHREX_ALIGNED_SP1_ELF_PATH`: Path to the SP1 ELF program. This is the same file used for SP1 verification outside of Aligned mode.
- `ETHREX_ALIGNED_NETWORK` and `ETHREX_ALIGNED_FEE_ESTIMATE`: Parameters used by the [Aligned SDK](https://docs.alignedlayer.com/guides/1.2_sdk_api_reference).

### 4. Running the Prover

In a console with `ethrex/crates/l2` as the current directory, run the following command:

```bash
SP1_PROVER=cuda make init-prover PROVER=sp1 PROVER_CLIENT_ALIGNED=true
```


## Behavioral Differences in Aligned Mode

### Prover

- Generates `Compressed` proofs instead of `Groth16`.
- Required because Aligned currently only accepts SP1 compressed proofs.

### Proof Sender

- Sends proofs to the **Aligned Batcher** instead of the `OnChainProposer` contract.
- Tracks the last proof sent using the rollup store.

![Proof Sender Aligned Mode](img/aligned_mode_proof_sender.png)

### Proof Verifier

- Only spawned in Aligned mode.
- Monitors whether the next proof has been aggregated by Aligned.
- Once verified, it triggers the advancement of the `OnChainProposer` contract.

![Aligned Mode Proof Verifier](img/aligned_mode_proof_verifier.png)

### OnChainProposer

- Uses `verifyBatchAligned()` instead of `verifyBatch()`.
- Delegates proof verification to the `AlignedProofAggregatorService` contract.
- Currently supports one proof per transaction.
- Future updates aim to support verifying an array of proofs in a single call.
