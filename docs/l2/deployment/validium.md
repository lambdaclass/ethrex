# Deploying a validium ethrex L2

In this section, we'll cover how to deploy a validium ethrex L2 on a public network such as Holesky, Sepolia, or Mainnet.

## Prerequisites

This guide assumes that you have ethrex installed and available in your PATH. If you haven't installed it yet, follow one of the methods in the Installation Guide. If you want to build the binary from source, refer to the [Building from source](./overview.md#building-from-source-skip-if-ethrex-is-already-installed) section and select the appropriate build option.

## 1. Deploy the Contracts

First, deploy and initialize the contracts on L1 using the ethrex l2 deploy command (for more details on the ethrex CLI, see the ethrex CLI Reference section):

```shell
ethrex l2 deploy \
  --validium true \
  --eth-rpc-url <L1_RPC_URL> \
  --private-key <PRIVATE_KEY> \
  --genesis-l2-path <PATH_TO_L2_GENESIS_FILE> \
  --bridge-owner <COMMON_BRIDGE_OWNER_ADDRESS> \
  --on-chain-proposer-owner <ON_CHAIN_PROPOSER_OWNER_ADDRESS> \
  --committer.l1-address <L1_COMMITTER_ADDRESS> \
  --proof-sender.l1-address <L1_PROOF_SENDER_ADDRESS> \
  --env-file-path <PATH_TO_ENV_FILE> \
  --randomize-contract-deployment
```

> [!CAUTION]
> Ensure you control the Committer and Proof Sender accounts, as they will be authorized as sequencers. These accounts will have control over the chain state.

> [!IMPORTANT]
> If you plan to prove your L2 using SP1, RISC0, or TEE, add the following extra arguments to the command above:
>
> `--sp1 true` to require SP1 proofs for validating batch execution and state settlement.
> 
> `--sp1.verifier-address` to use an existing verifier instead of deploying one on the public network. Succinct Labs recommends their deployed canonical verifier gateways; see the list here.
> 
> `--risc0 true` to require RISC0 proofs for validating batch execution and state settlement.
> 
> `--risc0.verifier-address` to use an existing verifier instead of deploying one on the public network. RISC0 recommends their deployed canonical verifier gateways; see the list here.
> 
> `--tdx true` to require TEE proofs for validating batch execution and state settlement.
> 
> `--tdx.verifier-address` to use an existing verifier instead of deploying one on the public network. Do not pass this flag if you want to deploy a new verifier.
> 
> Enabling multiple proving backends will require running multiple provers, one for each backend. Refer to the [Run multiple provers](./prover/multi-prover.md) section for more details.
> 
> If you enable more than one proving system (e.g., both `--sp1 true` and `--risc0 true`), all selected proving systems will be required (i.e., every batch must include a proof from each enabled system to settle on L1).

> [!IMPORTANT]
> Retrieve the deployed contract addresses from the console logs or the .env file generated during deployment (in the directory where you ran the command) for use in the next step.

> [!NOTE]
>
> - Replace `L1_RPC_URL` with your preferred RPC provider endpoint.
> - Replace `PRIVATE_KEY` with the private key of an account funded on the target L1. This key will sign the transactions during deployment.
> - Replace `PATH_TO_L2_GENESIS_FILE` with the path to your L2 genesis file. A genesis example is available in the fixtures directory of the [official GitHub repository](https://github.com/lambdaclass/ethrex/blob/main/fixtures/genesis/l2.json). This file initializes the `OnChainProposer` contract with the genesis state root.
> - The `CommonBridge` and `OnChainProposer` contracts are upgradeable and ownable, with implementations behind proxies initialized during deployment. Replace `COMMON_BRIDGE_OWNER_ADDRESS` and `ON_CHAIN_PROPOSER_OWNER_ADDRESS` with the address of the account you want as the owner. The owner can upgrade implementations or perform administrative actions; for more details, see the Architecture section.
> - The sequencer components (`L1Committer` and `L1ProofSender`) require funded accounts on the target L1 to advance the network. Replace `L1_COMMITTER_ADDRESS` and `L1_PROOF_SENDER_ADDRESS` with the addresses of those accounts.
> - Replace `PATH_TO_ENV_FILE` with the path where you want to save the generated environment file. This file contains the deployed contract addresses and other configuration details needed to run the L2 node.
> - L1 contract deployment uses the `CREATE2` opcode for deterministic addresses. To deploy non-deterministically, include the `--randomize-contract-deployment` flag.

## 2. Start the L2 node

Once the contracts are deployed, start the L2 node:

```shell
ethrex l2 \
  --validium \
  --l1.bridge-address <COMMON_BRIDGE_ADDRESS> \
  --l1.on-chain-proposer-address <ON_CHAIN_PROPOSER_ADDRESS> \
  --block-producer.coinbase-address <L2_COINBASE_ADDRESS> \
  --proof-coordinator.l1-private-key <L1_PROOF_SENDER_PRIVATE_KEY> \
  --committer.l1-private-key <L1_COMMITTER_PRIVATE_KEY> \
  --eth.rpc-url <L1_RPC_URL> \
  --network <PATH_TO_L2_GENESIS_FILE> \
  --no-monitor
```

> [!CAUTION]
> Replace `L1_COMMITTER_PRIVATE_KEY` and `L1_PROOF_SENDER_PRIVATE_KEY` with the private keys for the `L1_COMMITTER_ADDRESS` and `L1_PROOF_SENDER_ADDRESS` used in the deployment step, respectively.

> [!IMPORTANT]
>
> The L1 Committer and L1 Proof Sender accounts must be funded for the chain to advance.

> [!NOTE]
>
> - Replace `COMMON_BRIDGE_ADDRESS` and `ON_CHAIN_PROPOSER_ADDRESS` with the proxy addresses for the CommonBridge and OnChainProposer contracts from the deployment step.
> - Replace `L2_COINBASE_ADDRESS` with the address that will collect L2 block fees. To access these funds on L1, you'll need to withdraw them (see the Withdrawals section for details).
> - Replace `L1_PROOF_SENDER_PRIVATE_KEY` and `L1_COMMITTER_PRIVATE_KEY` with the private keys for the `L1_PROOF_SENDER_ADDRESS` and `L1_COMMITTER_ADDRESS` from the deployment step.
> - Replace `L1_RPC_URL` and `PATH_TO_L2_GENESIS_FILE` with the same values used in the deployment step.
> - Tune throughput with the gas caps:
>   - `--block-producer.block-gas-limit` (env: `ETHREX_BLOCK_PRODUCER_BLOCK_GAS_LIMIT`, default: `30000000`): Sets the gas limit per L2 block.
>   - `--committer.batch-gas-limit` (env: `ETHREX_COMMITTER_BATCH_GAS_LIMIT`): Sets the gas limit per batch sent to L1—should be at or above the block gas limit.
> 
>   You can use either the environment variables or the flags to configure these values.

That's it! You now have a validium ethrex L2 up and running. However, one key component is still missing: state proving. The L2 state is considered final only after a batch execution ZK proof is successfully verified on-chain. Generating these proofs requires running a dedicated prover, which is covered in the Run an ethrex L2 Prover section.
