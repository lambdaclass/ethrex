# Deploying a node

## Prerequisites

This guide assumes that you've deployed the contracts for the rollup to your chosen L1 network, and that you have a valid `genesis.json`.
The contract's solidity code can be downloaded from the [GitHub releases](https://github.com/lambdaclass/ethrex/releases)
or by running:

```
curl -L https://github.com/lambdaclass/ethrex/releases/latest/download/ethrex-l1-contracts.tar.gz
```

## Starting the sequencer

First we need to set some environment variables.

#### Set the OnChainProposer address:

```sh
export ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS=<address>
```

#### Set the L1 bridge address

```sh
export ETHREX_WATCHER_BRIDGE_ADDRES=<address>
```

#### Set the L1 RPC endpoint url

```sh
export ETHREX_ETH_RPC_URL=<url>
```

#### Set the committer private key

This is the private key of the address that will send `commitBatch` transactions.

```sh
export ETHREX_COMMITTER_L1_PRIVATE_KEY=<pk>
```

#### Set the verifier private key

This is the private key of the address that will send `verifyBatch` transactions.

```sh
export ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY=<pk>
```

#### Run the sequencer

```sh
    ethrex l2 \
	--network <path-to-your-genesis.json> \
	--block-producer.coinbase-address <l2-coinbase-address> \
```

This will start an ethrex l2 sequencer with the RPC server listening at `http://localhost:1729` and the proof coordinator server listening at `http://localhost:3900`

## Starting a prover server

```sh
ethrex l2 prover --proof-coordinator http://localhost:3900
```
