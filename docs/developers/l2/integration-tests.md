# Running integration tests

In this section, we will explain how to run integration tests for ethrex L2 with the objective of validating the correct functioning of our stack in our releases. For this, we will use ethrex as a local L2 dev node.

## Prerequisites

- This guide assumes that you have ethrex L2 installed. If you haven't done so, follow one of the installation methods in the [installation guide](https://docs.ethrex.xyz/getting-started/installation/).
- For running the tests, you'll need a fresh clone of [ethrex](https://github.com/lambdaclass/ethrex/).

## Setting up the environment

Our integration tests assume that there is an ethrex L1 node, an ethrex L2 node, and an ethrex L2 prover up and running. So before running them, we need to start the nodes.

### Running ethrex L2 dev node

For this, we are using the `ethrex l2 --dev` command, which does this job for us. In one console, run the following:

```
ethrex l2 --dev \
--committer.commit-time 150000 \
--block-producer.block-time 1000
```

> [!NOTE]
> ethrex's MPT implementation is path-based, and the database commit threshold is set to `128`. In simple words, the latter implies that the database only stores the state 128 blocks before the current one (e.g., if the current block is block 256, then the database stores the state at block 128), while the state of the blocks within lives in in-memory diff layers (which are lost during node shutdowns).
> In ethrex L2, this has a direct impact since if our sequencer seals batches with more than 128 blocks, it won't be able to retrieve the state previous to the first block of the batch being sealed because it was pruned; therefore, it won't be able to commit.
> To solve this, after a batch is sealed, we create a checkpoint of the database at that point to ensure the state needed at the time of commitment is available for the sequencer.
> For this test to be valuable, we need to ensure this edge case is covered. To do so, we set up an L2 with batches of approximately 150 blocks. We achieve this by setting the flag `--block-producer.block-time` to 1 second, which specifies the interval in milliseconds for our builder to build an L2 block. This means the L2 block builder will build blocks every 1 second. We also set the flag `--committer.commit-time` to 150 seconds (2 minutes and 30 seconds), which specifies the interval in milliseconds in which we want to commit to the L1. This ensures that enough blocks are included in each batch.

So far, we have an ethrex L1 and an ethrex L2 node up and running. We only miss the ethrex L2 prover, which we are going to spin up in `exec` mode, meaning that it won't generate ZK proofs.

### Running ethrex L2 prover

In another terminal, run the following to spin up an ethrex L2 prover in exec mode:

```
ethrex l2 prover \
--backend exec \
--proof-coordinators http://localhost:3900
```

> [!NOTE]  
> The flag `--proof-coordinators` is used to specify one or more proof coordinator URLs. This is so because the prover is capable of proving ethrex L2 batches from multiple sequencers. We are particularly setting it to `localhost:3900` because the `ethrex l2 --dev` command uses the port `3900` for the proof coordinator by default.  
> To see more about the proof coordinator, read the [ethrex L2 sequencer](https://docs.ethrex.xyz/l2/architecture/sequencer.html#ethrex-l2-sequencer) and [ethrex L2 prover](https://docs.ethrex.xyz/l2/architecture/prover.html#ethrex-l2-prover) sections.

## Running the integration tests

During the execution of `ethrex l2 --dev`, a `.env` file is created and filled with environment variables containing contract addresses. This `.env` file is always needed for dev environments, so we need it for running the integration tests. Therefore, before running the integration tests, copy the `.env` file into `ethrex/cmd`:

```
cp .env ethrex/cmd
```

Finally, in another terminal (should be a third one at this point), change your current directory to `ethrex/crates/l2` and run:

```
make test
```

## Troubleshooting

TODO
