# Upgrade an L2 chain

This guide explains how to safely upgrade an Ethrex-based L2 chain. It covers pausing contracts, upgrading on-chain contracts, replacing node binaries, and unpausing.

> Note: This page references only commands and contracts that already exist in the repository. For background, see `docs/l2/deploy.md` and the L1/L2 contract sources.

## Prerequisites

- L1 RPC URL to the network where your rollup is deployed
- Owner account capable of administering the L1 contracts
- Addresses of your deployed contracts (at minimum the `OnChainProposer` and `CommonBridge` on L1)
- New node binaries or container images prepared for rollout

## 1) Prepare binaries

Follow the installation docs to obtain the new `ethrex` binaries or images:

- `docs/getting-started/installation/`
  - Binary distribution
  - Package manager
  - Docker image
  - Building from source

Ensure you can roll back to the previous version if needed.

## 2) Pause sequencing on L1

Pause sequencing to prevent new batches while upgrading. The `ethrex` CLI exposes pause/unpause helpers that call the target L1 contract method.

Pause the `OnChainProposer` contract on L1:

```sh
ethrex l2 pause \
  <ONCHAIN_PROPOSER_ADDRESS> \
  --private-key <OWNER_PRIVATE_KEY> \
  --rpc-url <L1_RPC_URL>
```

- This uses the `pause()` selector defined in the contracts:
  - `OnChainProposer.pause()` and `unpause()` exist and are owner-gated
  - `CommonBridge.pause()` and `unpause()` exist and are owner-gated

References:
- CLI implementation for Pause/Unpause lives in `cmd/ethrex/l2/command.rs` (selectors `pause()` / `unpause()`).
- Contract interfaces in `crates/l2/contracts/src/l1/` include these methods.

## 3) Upgrade L2 contracts (if applicable)

If your upgrade includes contract changes, use the L1 `CommonBridge` to upgrade L2 proxy implementations via L1-to-L2 message. The contract provides:

- `CommonBridge.upgradeL2Contract(address l2Contract, address newImplementation, uint256 gasLimit, bytes data)`
  - Sends a message to the L2 `TransparentUpgradeableProxy` admin to `upgradeToAndCall(newImplementation, data)` with the provided `data`.

Steps:
- Identify the L2 proxy address you intend to upgrade.
- Prepare the new implementation address on L2.
- Choose a sufficient `gasLimit` for the L2 execution.
- Encode any initializer `data` required by the new implementation (can be empty `0x`).
- From the owner account, call `upgradeL2Contract` on the L1 `CommonBridge` contract.

Note: There is no dedicated CLI subcommand for `upgradeL2Contract`; use your usual Ethereum tools/wallets to send this L1 transaction to the `CommonBridge` address with the arguments above. Contract source: `crates/l2/contracts/src/l1/CommonBridge.sol`.

## 4) Replace node binaries and restart services

Replace the sequencer and related service binaries/images with the new version and restart them. For process management and admin endpoints, see `docs/l2/admin.md`.

Typical flow:
- Stop the sequencer and related services
- Replace binaries/images
- Start services and verify healthy status and logs

## 5) Unpause sequencing

Once contracts and nodes are upgraded and verified, unpause the `OnChainProposer`:

```sh
ethrex l2 unpause \
  <ONCHAIN_PROPOSER_ADDRESS> \
  --private-key <OWNER_PRIVATE_KEY> \
  --rpc-url <L1_RPC_URL>
```

## Optional: Revert unverified batches

If you need to roll back unverified batches as part of the upgrade process, use the provided subcommand. This can pause, revert on-chain state via `OnChainProposer.revertBatch`, and prune local storage:

```sh
ethrex l2 revert-batch \
  <BATCH_NUMBER> \
  --datadir <DATA_DIR> \
  --rpc-url <L1_RPC_URL> \
  --owner-private-key <OWNER_PRIVATE_KEY> \
  --sequencer-private-key <SEQUENCER_PRIVATE_KEY> \
  --pause \
  --network <GENESIS_L2_PATH> \
  --delete-blocks \
  <ONCHAIN_PROPOSER_ADDRESS>
```

- Pauses before and unpauses after if `--pause` is provided
- Calls `revertBatch(uint256)` on `OnChainProposer` (only for unverified batches)
- Deletes batches from the rollup store and optionally deletes L2 blocks from the node store

Contract references:
- `OnChainProposer.revertBatch(uint256)` requires the contract to be paused and reverts only unverified batches.

---

After completion, monitor the system (metrics and logs) to ensure the chain progresses as expected. See `docs/l2/monitoring.md` for guidance.
