# Running Ethrex in Aligned Mode

This guide extends the [Deploy an L2 overview](./overview.md) and shows how to run an ethrex L2 with **Aligned mode** enabled.

> **For a comprehensive technical deep-dive into the Aligned integration architecture, see [Aligned Layer Integration](../fundamentals/ethrex_l2_aligned_integration.md).**

It assumes:

- You already installed the `ethrex` binary to your `$PATH` (for example from the repo root with `cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql --force`).
- You have the ethrex repository checked out locally for the `make` targets referenced below.

- Check [How to Run (local devnet)](#how-to-run-local-devnet) for development or testing.
- Check [How to Run (testnet)](#how-to-run-testnet) for a prod-like environment.

## How to run (testnet)

> [!IMPORTANT]
> This guide assumes there is an L1 running with all Aligned environment set.

### 1. Generate the prover ELF/VK

From the ethrex repository root run:

```bash
make -C crates/l2 build-prover-<sp1/risc0> # optional: GPU=true
```

This will generate the SP1 ELF program and verification key under:

- `crates/l2/prover/src/guest_program/src/sp1/out/riscv32im-succinct-zkvm-elf`
- `crates/l2/prover/src/guest_program/src/sp1/out/riscv32im-succinct-zkvm-vk-u32`

### 2. Deploying L1 Contracts

Run the deployer with the Aligned settings:

```bash
COMPILE_CONTRACTS=true \
ETHREX_L2_ALIGNED=true \
ETHREX_DEPLOYER_ALIGNED_AGGREGATOR_ADDRESS=<ALIGNED_AGGREGATOR_ADDRESS> \
ETHREX_L2_SP1=true \
ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT=true \
ethrex l2 deploy \
  --eth-rpc-url <ETH_RPC_URL> \
  --private-key <YOUR_PRIVATE_KEY> \
  --on-chain-proposer-owner <ON_CHAIN_PROPOSER_OWNER>  \
  --bridge-owner <BRIDGE_OWNER_ADDRESS>  \
  --genesis-l2-path fixtures/genesis/l2.json \
  --proof-sender.l1-address <PROOF_SENDER_L1_ADDRESS>
```

> [!NOTE]
> This command requires the `COMPILE_CONTRACTS` env variable to be set, as the deployer needs the SDK to embed the proxy bytecode.
> In this step we are initializing the `OnChainProposer` contract with the `ALIGNED_PROOF_AGGREGATOR_SERVICE_ADDRESS` and skipping the rest of verifiers; you can find the address for the aligned aggregator service [here](https://docs.alignedlayer.com/guides/7_contract_addresses).
> Save the addresses of the deployed proxy contracts, as you will need them to run the L2 node.
> Accounts for the deployer, on-chain proposer owner, bridge owner, and proof sender must have funds. Add `--bridge-owner-pk <PRIVATE_KEY>` if you want the deployer to immediately call `acceptOwnership` on behalf of that owner; otherwise, they can accept later.
> If you enable more than one proving system (e.g., both `--sp1 true` and `--risc0 true`), all selected proving systems will be required (i.e., every batch must include a proof from each enabled system to settle on L1).

### 3. Deposit funds to the `AlignedBatcherPaymentService` contract from the proof sender

```bash
aligned deposit-to-batcher \
  --network <NETWORK> \
  --private_key <PROOF_SENDER_PRIVATE_KEY> \
  --rpc_url <RPC_URL> \
  --amount <DEPOSIT_AMOUNT>
```

> [!IMPORTANT]
> Using the [Aligned CLI](https://docs.alignedlayer.com/guides/9_aligned_cli)

### 4. Running a node

Run the sequencer using the installed `ethrex` binary:

```bash
ethrex l2 \
  --watcher.block-delay 0 \
  --network fixtures/genesis/l2.json \
  --l1.bridge-address <BRIDGE_ADDRESS> \
  --l1.on-chain-proposer-address <ON_CHAIN_PROPOSER_ADDRESS> \
  --eth.rpc-url <ETH_RPC_URL> \
  --aligned \
  --aligned-network <ALIGNED_NETWORK>  \
  --block-producer.coinbase-address <COINBASE_ADDRESS>  \
  --committer.l1-private-key <COMMITTER_PRIVATE_KEY>  \
  --proof-coordinator.l1-private-key <PROOF_COORDINATOR_PRIVATE_KEY>  \
  --aligned.beacon-url <ALIGNED_BEACON_URL> \
  --datadir ethrex_l2 \
  --no-monitor
```

Both committer and proof coordinator should have funds.

Aligned params explanation:

- `--aligned`: Enables aligned mode, enforcing all required parameters.
- `--aligned.beacon-url`: URL of the beacon client used by the Aligned SDK to verify proof aggregations, it has to support `/eth/v1/beacon/blobs`
- `--aligned-network`: Parameter used by the [Aligned SDK](https://docs.alignedlayer.com/guides/1.2_sdk_api_reference).

If you can't find a beacon client URL which supports that endpoint, you can run your own with lighthouse and ethrex:

Create secrets directory and jwt secret

```bash
mkdir -p ethereum/secrets/
openssl rand -hex 32 | tr -d "\n" | tee ./ethereum/secrets/jwt.hex
```

```bash
lighthouse bn --network <NETWORK> --execution-endpoint http://localhost:8551 --execution-jwt ./ethereum/secrets/jwt.hex --checkpoint-sync-url <CHECKPOINT_URL> --http --purge-db-force --supernode
```

```bash
ethrex --authrpc.jwtsecret ./ethereum/secrets/jwt.hex --network <NETWORK>
```

### 5. Running the Prover

In another terminal start the prover(s):

```bash
make -C crates/l2 init-prover-<sp1/risc0> GPU=true # The GPU parameter is optional
```

Then you should wait until Aligned aggregates your proof.

## How to run (local devnet)

> [!IMPORTANT]
> This guide assumes you have already generated the prover ELF/VK. See: [Generate the prover ELF/VK](#1-generate-the-prover-elfvk)

### Set Up the Aligned Environment

1. Clone the Aligned repository and checkout the currently supported release:

    ```bash
    git clone git@github.com:yetanotherco/aligned_layer.git
    cd aligned_layer
    git checkout tags/v0.19.1
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

    Change `ethereum-genesis-generator` to 5.0.8

    ```
    ethereum_genesis_generator_params:
      # The image to use for ethereum genesis generator
      image: ethpandaops/ethereum-genesis-generator:5.0.8
    ```

3. Make sure you have the latest version of [kurtosis](https://github.com/kurtosis-tech/kurtosis) installed and start the ethereum-package:

    ```
    cd aligned_layer
    make ethereum_package_start
    ```

    If you need to stop it run `make ethereum_package_rm`

4. Start the batcher:

    First, increase the `max_proof_size` in `aligned_layer/config-files/config-batcher-ethereum-package.yaml` `max_proof_size: 104857600 # 100 MiB` for example.

    ```
    cd aligned_layer
    make batcher_start_ethereum_package
    ```

    This is the Aligned component that receives the proofs before sending them in a batch.

> [!WARNING]
> If you see the following error in the batcher: `[ERROR aligned_batcher] Unexpected error: Space limit exceeded: Message too long: 16940713 > 16777216` modify the file `aligned_layer/batcher/aligned-batcher/src/lib.rs` at line 433 with the following code:
>
> ```Rust
> use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
>
> let mut stream_config = WebSocketConfig::default();
> stream_config.max_frame_size = None;
>
> let ws_stream_future =
>     tokio_tungstenite::accept_async_with_config(raw_stream, Some(stream_config));
> ```

### Initialize L2 node

1. Deploy the L1 contracts, specifying the `AlignedProofAggregatorService` contract address, and adding the required prover types (Risc0 or SP1):

    ```bash
    COMPILE_CONTRACTS=true \
    ETHREX_L2_ALIGNED=true \
    ETHREX_DEPLOYER_ALIGNED_AGGREGATOR_ADDRESS=0xcbEAF3BDe82155F56486Fb5a1072cb8baAf547cc \
    ETHREX_L2_SP1=true \
    ETHREX_L2_RISC0=true \
    make -C crates/l2 deploy-l1
    ```

    Both `ETHREX_L2_SP1` and `ETHREX_L2_RISC0` are optional.

    > [!NOTE]
    > This command requires the COMPILE_CONTRACTS env variable to be set, as the deployer needs the SDK to embed the proxy bytecode.

    You will see that some deposits fail with the following error:

    ```
    2025-10-13T19:44:51.600047Z ERROR ethrex::l2::deployer: Failed to deposit address=0x0002869e27c6faee08cca6b765a726e7a076ee0f value_to_deposit=0
    2025-10-13T19:44:51.600114Z  WARN ethrex::l2::deployer: Failed to make deposits: Deployer EthClient error: eth_sendRawTransaction request error: insufficient funds for gas * price + value: have 0 want 249957710190063
    ```

    This is because not all the accounts are pre-funded from the genesis.

2. Send some funds to the Aligned batcher payment service contract from the proof sender:

    ```bash
    cargo run --manifest-path aligned_layer/crates/cli/Cargo.toml deposit-to-batcher \
      --network devnet \
      --private_key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
      --amount 1ether
    ```

3. Start the L2 node:

    ```bash
    ETHREX_ALIGNED_MODE=true \
    ETHREX_ALIGNED_BEACON_URL=http://127.0.0.1:58801 \
    ETHREX_ALIGNED_NETWORK=devnet \
    ETHREX_PROOF_COORDINATOR_DEV_MODE=false \
    SP1=true \
    RISC0=true \
    make -C crates/l2 init-l2
    ```

    Suggestion:
    When running the integration test, consider increasing the `--committer.commit-time` to 2 minutes. This helps avoid having to aggregate the proofs twice. You can do this by adding the following flag to the `init-l2-no-metrics` target:

    ```
    --committer.commit-time 120000
    ```

4. Start prover(s) in different terminals:

    ```bash
    make -C crates/l2 init-prover-<sp1/risc0> GPU=true # The GPU flag is optional
    ```

### Aggregate proofs:

After some time, you will see that the `l1_proof_verifier` is waiting for Aligned to aggregate the proofs. You can trigger an aggregation (for either sp1 or risc0 proofs) by running:

```bash
make -C aligned_layer proof_aggregator_start AGGREGATOR=<sp1/risc0>

# or with gpu acceleration
make -C aligned_layer proof_aggregator_start_gpu AGGREGATOR=<sp1/risc0>
```

If successful, the `l1_proof_verifier` will print the following logs:

```
INFO ethrex_l2::sequencer::l1_proof_verifier: Proof for batch 1 aggregated by Aligned with commitment 0xa9a0da5a70098b00f97d96cee43867c7aa8f5812ca5388da7378454580af2fb7 and Merkle root 0xa9a0da5a70098b00f97d96cee43867c7aa8f5812ca5388da7378454580af2fb7
INFO ethrex_l2::sequencer::l1_proof_verifier: Batches verified in OnChainProposer, with transaction hash 0x731d27d81b2e0f1bfc0f124fb2dd3f1a67110b7b69473cacb6a61dea95e63321
```

## Behavioral Differences in Aligned Mode

### Prover

- Generates `Compressed` proofs instead of `Groth16`.
- Required because Aligned accepts compressed proofs (both SP1 and RISC0).

### Proof Sender

- Sends proofs to the **Aligned Batcher** instead of the `OnChainProposer` contract.
- Tracks the last proof sent using the rollup store.

![Proof Sender Aligned Mode](../img/aligned_mode_proof_sender.png)

### Proof Verifier

- Spawned only in Aligned mode.
- Monitors whether the next proof has been aggregated by Aligned.
- Once verified, collects all already aggregated proofs and triggers the advancement of the `OnChainProposer` contract by sending a single transaction.

![Aligned Mode Proof Verifier](../img/aligned_mode_proof_verifier.png)

### OnChainProposer

- Uses `verifyBatchesAligned()` instead of `verifyBatch()`.
- Receives an array of proofs to verify.
- Delegates proof verification to the `AlignedProofAggregatorService` contract.

## Failure Recovery

For guidance on handling Aligned Layer failures and outages, see the [Aligned Failure Recovery Guide](./aligned_failure_recovery.md).
