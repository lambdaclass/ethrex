# Configuring gas limits

ethrex exposes two knobs to control how much L2 gas can be used per block and per batch. Making these limits explicit helps you balance throughput, proving time, and the cost of submitting batches to Ethereum.

- `--block-producer.block-gas-limit` (env: `ETHREX_BLOCK_PRODUCER_BLOCK_GAS_LIMIT`): caps the gas available for each L2 block produced by the sequencer. Default: `30000000`.
- `--committer.batch-gas-limit` (env: `ETHREX_COMMITTER_BATCH_GAS_LIMIT`): caps the total gas of the L2 transactions included in a batch sent to L1. Keep it at or above your L2 block gas limit so a full block always fits in a batch.

Raising the L2 block gas limit increases throughput but also increases proving time and proof size for every block. If you enable GPU/CPU proving with both `sp1` and `risc0`, remember that heavier blocks will lengthen proving time for both systems.

You can set the limits with CLI flags or environment variables. Example:

```bash
ETHREX_BLOCK_PRODUCER_BLOCK_GAS_LIMIT=40000000 \
ETHREX_COMMITTER_BATCH_GAS_LIMIT=40000000 \
ethrex l2 \
  --network fixtures/genesis/l2.json \
  --l1.bridge-address <BRIDGE_ADDRESS> \
  --l1.on-chain-proposer-address <ON_CHAIN_PROPOSER_ADDRESS> \
  --eth.rpc-url <ETH_RPC_URL> \
  --committer.l1-private-key <COMMITTER_PRIVATE_KEY> \
  --proof-coordinator.l1-private-key <PROOF_COORDINATOR_PRIVATE_KEY>
```

Adjust the values to fit your environment:

- Lower limits for local devnets or constrained prover hardware.
- Higher limits when you need more throughput and your proving/batching infrastructure can sustain the extra load.
