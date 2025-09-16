# Deploy an L2

## Prerequisites

This guide assumes that you have ethrex installed. If you haven't done so, follow one of the installation methods in the [installation guide](../getting-started/installation/).

## Deploy the contracts

The first step is to deploy the rollup's core contracts to your chosen L1 network.

### 1. Download the contracts

You can get the contracts in two ways:

- **From GitHub Releases:**

  - Download the latest release from [GitHub Releases](https://github.com/lambdaclass/ethrex/releases/download/v0.0.4-alpha/ethrex-contracts.tar.gz).

- **From source code (latest version):**

  - Clone the repository:
    ```sh
    git clone https://github.com/lambdaclass/ethrex.git
    cd ethrex/crates/l2/contracts/src/l1
    ```

### 2. Deploy the contracts

You can deploy the contracts manually or using the built-in tool:

```sh
ethrex l2 deploy \
    --eth-rpc-url <L1_RPC_URL> \
    --private-key <DEPLOYER_PRIVATE_KEY> \
    --genesis-l2-path <GENESIS_L2_PATH> \
    --risc0.verifier-address <RISC0_VERIFIER_ADDRESS> \
    --sp1.verifier-address <SP1_VERIFIER_ADDRESS> \
    --tdx.verifier-address <TDX_VERIFIER_ADDRESS> \
    --aligned.aggregator-address <ALIGNED_AGGREGATOR_ADDRESS> \
    --on-chain-proposer-owner <OWNER_ADDRESS> \
    --bridge-owner <OWNER_ADDRESS> \
    --randomize-contract-deployment
```

You can find a genesis example in the [repo](https://github.com/lambdaclass/ethrex/blob/main/fixtures/genesis/l2.json).

Verifier addresses can be set to `0x00000000000000000000000000000000000000AA` in case you don't want to use some prover. The same applies to Aligned.

> [!TIP]
> You can start a local [development L1](../l1/running/configuration.md#dev-mode-localnet) network with `ethrex l1 --dev` and use its RPC URL for testing.

## Run the sequencer

Next step is to start the sequencer. This command will start all necessary components for the L2 network except the prover.

```sh
ethrex l2 \
	--network <GENESIS_L2_PATH> \
	--l1.on-chain-proposer-address <ON_CHAIN_PROPOSER_ADDRESS> \
	--l1.bridge-address <BRIDGE_ADDRESS> \
	--rpc_url <L1_RPC_URL> \
	--committer.l1-private-key <COMMITTER_PRIVATE_KEY> \
    --proof-coordinator.l1-private-key <PROOF_COORDINATOR_PRIVATE_KEY> \
	--block-producer.coinbase-address <L2_COINBASE_ADDRESS> \
```

OnChainProposer and CommonBridge addresses can be found in the `.env` file, generated during the deployment process. Committer and Proof coordinator accounts must have L1 funds, as they will need to pay for gas fees on the L1 network.

For further configuration take a look at the [CLI document](../CLI.md#ethrex-l2)

## Run the prover

Lastly, you need to start the prover. This command will start the prover component for the L2 network.

```sh
ethrex l2 prover --proof-coordinators tcp://localhost:3900 --backend exec
```

In this example, the `exec` backend is used, which means the prover will only execute the transactions but not generate proofs. This is fine for development as it's faster. You may look for other backends like SP1 and RISC0 in production.

For further configuration take a look at the [CLI document](../CLI.md#ethrex-l2-prover)

## Checking that everything is running

After starting the sequencer and prover, you can verify that your L2 node is running correctly:

- **Check the sequencer RPC:**

  You can request the latest block number:

  ```sh
  curl http://localhost:1729 \
  	-H 'content-type: application/json' \
  	-d '{"jsonrpc":"2.0","method":"eth_blockNumber","id":"1","params":[]}'
  ```

  The answer should be like this, and advance every 5 seconds:

  ```
  {"id":"1","jsonrpc":"2.0","result":"0x1"}
  ```

- **Check logs:**
  - Review the terminal output or log files for any errors or warnings.
  - After some time (1 minute by default) there should be a log from the L1 Committer informing a new batch is being sent to L1.
