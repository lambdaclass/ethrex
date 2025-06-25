# Running Ethrex in Aligned Mode

This document explains how to run an Ethrex L2 node in **Aligned mode** and highlights the key differences in component behavior compared to the default mode.

## How to Run

> [!IMPORTANT]  
> For this guide we assumed that there is an L1 running with all Aligned environment set.

### 1. Generate the SP1 ELF Program and Verification Key

Run:

```bash
cd ethrex/crates/l2
SP1_PROVER=cuda make build-prover PROVER=sp1 PROVER_CLIENT_ALIGNED=true
```

This will generate the SP1 ELF program and verification key under:
- `crates/l2/prover/zkvm/interface/sp1/out/riscv32im-succinct-zkvm-elf`
- `crates/l2/prover/zkvm/interface/sp1/out/riscv32im-succinct-zkvm-vk`


### 2. Deploying L1 Contracts

In a console with `ethrex/crates/l2` as the current directory, run the following command:

```bash
cargo run --release --bin ethrex_l2_l1_deployer --manifest-path contracts/Cargo.toml -- \
	--eth-rpc-url <L1_RPC_URL> \
	--private-key <L1_PRIVATE_KEY> \
	--genesis-l1-path <GENESIS_L1_PATH> \
	--genesis-l2-path <GENESIS_L2_PATH> \
	--contracts-path contracts \
	--sp1.verifier-address 0x00000000000000000000000000000000000000aa \
	--risc0.verifier-address 0x00000000000000000000000000000000000000aa \
	--tdx.verifier-address 0x00000000000000000000000000000000000000aa \
    --aligned.aggregator-address <ALIGNED_PROOF_AGGREGATOR_SERVICE_ADDRESS> \
    --bridge-owner <ADDRESS> \
    --on-chain-proposer-owner <ADDRESS> \
    --private-keys-file-path <PRIVATE_KEYS_FILE_PATH> \
    --sequencer-registry-owner <ADDRESS> \
    --sp1-vk-path <SP1_VERIFICATION_KEY_PATH>
```

> [!NOTE]
> In this step we are initiallizing the `OnChainProposer` contract with the `ALIGNED_PROOF_AGGREGATOR_SERVICE_ADDRESS` and skipping the rest of verifiers.  
> Save the addresses of the deployed proxy contracts, as you will need them to run the L2 node.

### 3. Deposit funds to the `AlignedBatcherPaymentService` contract from the proof sender

```bash
aligned \
--network <NETWORK> \
--private_key <PROOF_SENDER_PRIVATE_KEY> \
--amount <DEPOSIT_AMOUNT>
```

> [!IMPORTANT]
> Using the [Aligned CLI](https://docs.alignedlayer.com/guides/9_aligned_cli)

### 4. Running a node

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
	--l1.bridge-address <BRIDGE_ADDRESS> \
	--l1.on-chain-proposer-address <ON_CHAIN_PROPOSER_ADDRESS> \
	--eth.rpc-url <L1_RPC_URL> \
	--block-producer.coinbase-address <BLOCK_PRODUCER_COINBASE_ADDRESS> \
	--committer.l1-private-key <COMMITTER_PRIVATE_KEY> \
	--proof-coordinator.l1-private-key <PROOF_COORDINATOR_PRIVATE_KEY> \
	--proof-coordinator.addr <PROOF_COORDINATOR_ADDRESS> \
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

## How to Run Using an Aligned Dev Environment

> [!IMPORTANT]
> This guide asumes you have already generated the SP1 ELF Program and Verification Key. See: [Generate the SP1 ELF Program and Verification Key](#1-generate-the-sp1-elf-program-and-verification-key)


### Set Up the Aligned Environment

1. Clone the Aligned repository and checkout the currently supported release:

```bash
git clone git@github.com:yetanotherco/aligned_layer.git
cd aligned_layer
git checkout tags/v0.16.1
```

2. Edit the `aligned_layer/network_params.rs` file to send some funds to the `committer` and `integration_test` addresses:
```
prefunded_accounts: '{
    "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266": { "balance": "100000000000000ETH" },
    "0x70997970C51812dc3A010C7d01b50e0d17dc79C8": { "balance": "100000000000000ETH" },
    
    ...
    "0xa0Ee7A142d267C1f36714E4a8F75612F20a79720": { "balance": "100000000000000ETH" },
+   "0x4417092B70a3E5f10Dc504d0947DD256B965fc62": { "balance": "100000000000000ETH" },
+   "0x3d1e15a1a55578f7c920884a9943b3b35d0d885b": { "balance": "100000000000000ETH" },
     }'
```
You can also decrease the seconds per slot in `aligned_layer/network_params.rs`:
```
# Number of seconds per slot on the Beacon chain
  seconds_per_slot: 4
```
3. Make sure you have the latest version of [kurtosis](https://github.com/kurtosis-tech/kurtosis) installed and start the ethereum-package:

```
cd aligned_layer
make ethereum_package_start
```

To stop it run `make ethereum_package_rm` 

4. Start the batcher:

First, increase the `max_proof_size` in `aligned_layer/config-files/config-batcher-ethereum-package.yaml` `max_proof_size: 41943040` for example.

```
cd aligned_layer
make batcher_start_ethereum_package
```
This is the Aligned component that receives the proofs before sending them in a batch.
> [!Warning]
> If you see the following error in the batcher: `[ERROR aligned_batcher] Unexpected error: Space limit exceeded: Message too long: 16940713 > 16777216` modify the file `aligned_layer/batcher/aligned-batcher/src/lib.rs` at line 433 with the following code:
```Rust
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

let mut stream_config = WebSocketConfig::default();
stream_config.max_frame_size = None;

let ws_stream_future =
    tokio_tungstenite::accept_async_with_config(raw_stream, Some(stream_config));
```

### Initialize L2 node
1. In another terminal, let's deploy the L1 contracts specifying the `AlignedProofAggregatorService` contract address:
```
cd ethrex/crates/l2
cargo run --release --bin ethrex_l2_l1_deployer --manifest-path contracts/Cargo.toml -- \
	--eth-rpc-url http://localhost:8545 \
	--private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
	--contracts-path contracts \
	--risc0.verifier-address 0x00000000000000000000000000000000000000aa \
	--sp1.verifier-address 0x00000000000000000000000000000000000000aa \
	--tdx.verifier-address 0x00000000000000000000000000000000000000aa \
	--aligned.aggregator-address 0xFD471836031dc5108809D173A067e8486B9047A3 \
	--on-chain-proposer-owner 0x03d0a0aee676cc45bf7032649e0871927c947c8e \
	--bridge-owner 0x03d0a0aee676cc45bf7032649e0871927c947c8e \
	--deposit-rich \
	--private-keys-file-path ../../test_data/private_keys_l1.txt \
	--genesis-l1-path ../../test_data/genesis-l1-dev.json \
	--genesis-l2-path ../../test_data/genesis-l2.json
```

You will see that some deposits fail with the following error:
```
2025-06-18T19:19:24.066126Z  WARN ethrex_l2_l1_deployer: Failed to make deposits: Deployer EthClient error: eth_estimateGas request error: execution reverted: CommonBridge: amount to deposit is zero: CommonBridge: amount to deposit is zero
```

This is because not all the accounts are pre-funded from the genesis.

2. Send some funds to the Aligned batcher payment service contract from the proof sender:
```
cd aligned_layer/batcher/aligned
cargo run deposit-to-batcher \
--network devnet \
--private_key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
--amount 1ether
```

3. Start our l2 node:

```
cd ethrex/crates/l2
ETHREX_PROOF_COORDINATOR_DEV_MODE=false cargo run --release --manifest-path ../../Cargo.toml --bin ethrex --features "l2,rollup_storage_libmdbx,metrics" -- l2 init --watcher.block-delay 0 --network ../../test_data/genesis-l2.json --http.port 1729 --http.addr 0.0.0.0 --evm levm --datadir dev_ethrex_l2 --l1.bridge-address <BRIDGE_ADDRESS> --l1.on-chain-proposer-address <ON_CHAIN_PROPOSER_ADDRESS> --eth.rpc-url http://localhost:8545 --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d --committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 --proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d --proof-coordinator.addr 127.0.0.1 --aligned --aligned.beacon-url http://127.0.0.1:58801 --aligned-network devnet --aligned-sp1-elf-path prover/zkvm/interface/sp1/out/riscv32im-succinct-zkvm-elf
```

> [!IMPORTANT]  
> Set `BRIDGE_ADDRESS` and `ON_CHAIN_PROPOSER_ADDRESS` with the values printed in step 1.

Suggestion:
When running the integration test, consider increasing the `commit-time-ms` to 2 minutes. This helps avoid having to aggregate the proofs twice. You can do this by adding the following flag to the `init-l2-no-metrics` target:
```
--commit-time-ms 120000
```

4. Start prover:
```
cd ethrex/crates/l2
SP1_PROVER=cuda make init-prover PROVER=sp1 PROVER_CLIENT_ALIGNED=true
```

### Aggregate proofs:

After some time, you will see that the `l1_proof_verifier` is waiting for Aligned to aggregate the proofs:
```
2025-06-18T22:03:53.470356Z  INFO ethrex_l2::sequencer::l1_proof_verifier: Batch 1 has not yet been aggregated by Aligned. Waiting for 5 seconds
```

You can aggregate them by running:
```
cd aligned_layer
make start_proof_aggregator AGGREGATOR=sp1
```

If successful, the `l1_proof_verifier` will print the following logs:

```
INFO ethrex_l2::sequencer::l1_proof_verifier: Proof for batch 1 aggregated by Aligned with commitment 0xa9a0da5a70098b00f97d96cee43867c7aa8f5812ca5388da7378454580af2fb7 and Merkle root 0xa9a0da5a70098b00f97d96cee43867c7aa8f5812ca5388da7378454580af2fb7
INFO ethrex_l2::sequencer::l1_proof_verifier: Batch 1 verified in AlignedProofAggregatorService, with transaction hash 0x731d27d81b2e0f1bfc0f124fb2dd3f1a67110b7b69473cacb6a61dea95e63321
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
