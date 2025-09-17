# Deploying a node

## Prerequisites

This guide assumes that you've deployed the contracts for the rollup to your chosen L1 network, and that you have a valid `genesis.json`.
The contract's solidity code can be downloaded from the [GitHub releases](https://github.com/lambdaclass/ethrex/releases)
or by running:

```
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-contracts.tar.gz
```

## Starting the sequencer

First we need to set some environment variables.

#### Run the sequencer

```sh
    ethrex l2 \
	--network <path-to-your-genesis.json> \
	--on_chain_proposer_address <address> \
	--bridge_address <address> \
	--rpc_url <rpc-url> \
	--committer_l1_private_key <private-key> \
	--proof_coordinator_l1_private_key \
	--block-producer.coinbase-address <l2-coinbase-address> \
```

For further configuration take a look at the [CLI document](../CLI.md#ethrex-l2)

This will start an ethrex l2 sequencer with the RPC server listening at `http://localhost:1729` and the proof coordinator server listening at `http://localhost:3900`

## Starting a prover server

```sh
ethrex l2 prover --proof-coordinators http://localhost:3900
```

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

If all endpoints respond and there are no errors in the logs, your L2 node is running successfully.
