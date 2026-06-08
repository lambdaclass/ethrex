# Running integration tests

In this section, we will explain how to run integration tests for ethrex L2 with the objective of validating the correct functioning of our stack in our releases. For this, we will use ethrex as a local L2 dev node.

## Prerequisites

- Install the latest ethrex release or pre-release binary following the instructions in the [Install ethrex (binary distribution)](https://docs.ethrex.xyz/getting-started/installation/binary_distribution.html) section.
- For running the tests, you'll need a fresh clone of [ethrex](https://github.com/lambdaclass/ethrex/).
- (Optional for troubleshooting)
    - An Ethereum utility tool like [rex](https://github.com/lambdaclass/rex).
    - [`jq`](https://jqlang.org/download/) for JSON processing.
    - [`curl`](https://curl.se/download.html) for making HTTP requests.

## Setting up the environment

Our integration tests assume that there is an ethrex L1 node, an ethrex L2 node, and an ethrex L2 prover up and running. So before running them, we need to start the nodes.

### Running ethrex L2 dev node

For this, we are using the `ethrex l2 --dev` command, which does this job for us. In one console, run the following:

```shell
./ethrex l2 --dev \
--committer.commit-time 150000 \
--block-producer.block-time 1000 \
--block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
--block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
--block-producer.l1-fee-vault-address 0x45681AE1768a8936FB87aB11453B4755e322ceec \
--block-producer.operator-fee-per-gas 1000000000 \
--no-monitor
```

Read the note below for explanations about the flags used.

> [!NOTE]
> ethrex's MPT implementation is path-based, and the database commit threshold is set to `128`. In simple words, the latter implies that the database only stores the state 128 blocks before the current one (e.g., if the current block is block 256, then the database stores the state at block 128), while the state of the blocks within lives in in-memory diff layers (which are lost during node shutdowns).
> In ethrex L2, this has a direct impact since if our sequencer seals batches with more than 128 blocks, it won't be able to retrieve the state previous to the first block of the batch being sealed because it was pruned; therefore, it won't be able to create new batches to send to L1.
> To solve this, after a batch is sealed, we create a checkpoint of the database at that point to ensure the state needed at the time of commitment is available for the sequencer.
> For this test to be valuable, we need to ensure this edge case is covered. To do so, we set up an L2 with batches of approximately 150 blocks. We achieve this by setting the flag `--block-producer.block-time` to 1 second, which specifies the interval in milliseconds for our builder to build an L2 block. This means the L2 block builder will build blocks every 1 second. We also set the flag `--committer.commit-time` to 150 seconds (2 minutes and 30 seconds), which specifies the interval in milliseconds in which we want to commit to the L1. This ensures that enough blocks are included in each batch.
> The L2's gas pricing mechanism is tested in the integration tests, so we need to set the following flags to ensure the L2 gas pricing mechanism is active:
>
> - `--block-producer.base-fee-vault-address`
> - `--block-producer.operator-fee-vault-address`
> - `--block-producer.l1-fee-vault-address`
> - `--block-producer.operator-fee-per-gas`
>
> Read more about ethrex L2 gas pricing mechanism [here](https://docs.ethrex.xyz/l2/fundamentals/transaction_fees.html).
> We set the flag `--no-monitor` to disable the built-in monitoring dashboard since it is not needed for running the integration tests.

So far, we have an ethrex L1 and an ethrex L2 node up and running. We only miss the ethrex L2 prover, which we are going to spin up in `exec` mode, meaning that it won't generate ZK proofs.

### Running ethrex L2 prover

In another terminal, run the following to spin up an ethrex L2 prover in exec mode:

```shell
./ethrex l2 prover \
--backend exec \
--proof-coordinators http://localhost:3900
```

> [!NOTE]  
> The flag `--proof-coordinators` is used to specify one or more proof coordinator URLs. This is so because the prover is capable of proving ethrex L2 batches from multiple sequencers. We are particularly setting it to `localhost:3900` because the `ethrex l2 --dev` command uses the port `3900` for the proof coordinator by default.  
> To see more about the proof coordinator, read the [ethrex L2 sequencer](https://docs.ethrex.xyz/l2/architecture/sequencer.html#ethrex-l2-sequencer) and [ethrex L2 prover](https://docs.ethrex.xyz/l2/architecture/prover.html#ethrex-l2-prover) sections.

## Running the integration tests

During the execution of `ethrex l2 --dev`, a `.env` file is created and filled with environment variables containing contract addresses. This `.env` file is always needed for dev environments, so we need it for running the integration tests. Therefore, before running the integration tests, copy the `.env` file into `ethrex/cmd`:

```shell
cp .env ethrex/cmd
```

Finally, in another terminal (should be a third one at this point), change your current directory to `ethrex/crates/l2` and run:

```shell
make test
```

## FAQ

### What should I expect?

Once you run `make test`, you should see the output of the tests being executed one after another. The tests will interact with the ethrex L2 node and the ethrex L2 prover that you started previously. If everything is set up correctly, all tests should pass successfully.

### How long do the tests take to run?

The current configuration of the L2 node (with a block time of 1 second and a commit time of 150 seconds) means that each batch will contain approximately 150 blocks. Given this setup, the integration tests typically take around 30 to 45 minutes to complete, depending on the timing in which you performed the steps.

### I think my tests are taking too long, how can I debug this?

If your tests are taking significantly longer than expected, you are likely watching the `Retrying to get message proof for tx ...` counter in the tests terminal increase without progressing. Let's unveil what is happening here. This message indicates that the transaction has been included in an L2 block, but that block has not yet been included in a batch. There's no current way to fairly estimate when the block including the transaction will be included in a batch, but we can see how far the block is from being included.

Using the hash of the transaction shown in the log message, you can check the status of the transaction using an Ethereum utility tool like `rex`. Run the following commands in a new terminal:

1. Get the block number where the transaction was included (replace `<TX_HASH>` with the actual transaction hash):
   ```shell
   rex l2 tx <TX_HASH>
   ```
2. As the block is assumed to not be included in a batch yet, we need to check which blocks have been included in the latest batch. `rex` does not have a command for this yet, so we will use `curl` to make a JSON-RPC call to the ethrex L2 node. Run the following command:
   ```shell
   curl -X POST http://localhost:1729 \
   -H "Content-Type: application/json" \
   -d '{
   "jsonrpc":"2.0",
   "method":"ethrex_batchNumber",
   "params": [],
   "id":1
   }' | jq .result
   ```
3. Once you have the batch number, you can get the range of blocks included in that batch by running the following command (replace `<BATCH_NUMBER>` with the actual batch number obtained in the previous step, in hex format, e.g., `0x1`):
   ```shell
   curl -X POST http://localhost:1729 \
   -H "Content-Type: application/json" \
   -d '{
       "jsonrpc":"2.0",
       "method":"ethrex_getBatchByNumber",
       "params": ["<BATCH_NUMBER>", false],
       "id":1
   }' | jq .result.first_block,.result.last_block
   ```
4. Compare the block number obtained in step 1 with the range of blocks obtained in step 3 to see how far the block is from being included in a batch. To have a rough estimate, take into account the mean of blocks that are being included into the batches and consider that a batch is sealed approximately every 150 seconds (2 minutes and 30 seconds) based on the current configuration.

### Should I worry about the periodic warning logs of the L2 prover?

Logs are being constantly improved to provide better clarity. However, during the execution of the integration tests, you might notice periodic warning logs from the L2 prover indicating that there are no new batches to prove. These warnings are expected behavior in this testing scenario and can be safely ignored.

### The tests are failing, what should I do?

If the tests are failing, first ensure that both the ethrex L2 node and the ethrex L2 prover are running correctly without any errors. Check their logs for any issues. If everything seems fine, try restarting both services and rerun the tests. Ensure that your configuration files (e.g., `.env`) are correctly set up and that all required environment variables are defined. If the problem persists, consider reaching out to the ethrex community or support channels for further assistance.

### How do I run the tests with ethrex running from docker?

To run the integration tests with ethrex running on Docker, pass `--net=host` to docker run so it binds all ports in `localhost`:

```shell
docker run --net=host <other_docker_flags> ghcr.io/lambdaclass/ethrex:<tag>-l2 <ethrex_flags>
```

For ["Running ethrex L2 dev node"](#running-ethrex-l2-dev-node), with the latest image, the full command would be:

```shell
docker run --net=host ghcr.io/lambdaclass/ethrex:latest-l2 l2 --dev \
--committer.commit-time 150000 \
--block-producer.block-time 1000 \
--block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
--block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
--block-producer.l1-fee-vault-address 0x45681AE1768a8936FB87aB11453B4755e322ceec \
--block-producer.operator-fee-per-gas 1000000000 \
--no-monitor
```

For ["Running ethrex L2 prover"](#running-ethrex-l2-prover), with the latest image, the full command would be:

```shell
docker run --net=host ghcr.io/lambdaclass/ethrex:latest-l2 l2 prover \
--backend exec \
--proof-coordinators http://localhost:3900
```

## Troubleshooting

> [!NOTE]
> This is a placeholder for future troubleshooting tips.
> Please report any issues you encounter while running the integration tests to help us improve this section.
