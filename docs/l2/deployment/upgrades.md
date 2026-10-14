# Upgrades

## From v7 to v8

### Database migration (local node)

This migration applies to the L2 node database only (SQL-backed store). It does not change any on-chain contracts.

The `messages` table was renamed to `l1_messages`. Copy the data and then remove the old table:

```sql
INSERT INTO l1_messages
SELECT *
FROM messages;
```

Then delete the `messages` table.

## From v8 to v9

### Timelock upgrade (L1 contracts)

From ethrex v9 onwards, the Timelock contract manages access to the OnChainProposer (OCP). The OCP owner becomes the Timelock, and the deprecated `authorizedSequencerAddresses` mapping is replaced by Timelock roles.

#### What changes

- Sequencer permissions move to Timelock roles (`SEQUENCER`).
- Commit and verify transactions must target the Timelock, not the OCP.
- Governance and Security Council accounts control upgrades and emergency actions via the Timelock.

#### 1) Deploy Timelock (proxy + implementation)

Deploy a Timelock proxy and implementation using your standard UUPS deployment flow. Record the proxy address; that is the address you will initialize and use later.

#### 2) Initialize Timelock

Call the Timelock initializer on the proxy:

```
initialize(uint256 minDelay,address[] sequencers,address governance,address securityCouncil,address onChainProposer)
```

- `sequencers` should include the L1 committer and proof sender addresses (and any other accounts that should commit or verify).
- `securityCouncil` is typically the current OCP owner (ideally a multisig).
- `governance` is the account that will schedule and execute timelocked upgrades.

#### 3) Transfer OCP ownership to Timelock

From the current OCP owner, transfer ownership with `transferOwnership(address)`.

Then accept ownership from the Timelock:

- Normal path: schedule and execute `acceptOwnership()` through the Timelock (respects `minDelay`).
- Emergency path: the Security Council can call `emergencyExecute` on the Timelock with calldata for `acceptOwnership()` to accept immediately.

Note: `acceptOwnership()` must be executed by the Timelock (the pending owner), so it cannot be called directly from an EOA.

#### 4) Configure the L2 node to use Timelock

This is required because the sequencer can no longer call the OCP directly once the Timelock is the owner. Set the Timelock address so commit and verify calls target the Timelock:

- CLI flag: `--l1.timelock-address <TIMELOCK_PROXY_ADDRESS>`
- Env var: `ETHREX_TIMELOCK_ADDRESS=<TIMELOCK_PROXY_ADDRESS>`

The committer requires this address for non-based deployments, and the proof sender/verifier will use it when present.

Do this before restarting the sequencer after the ownership transfer. If the node keeps targeting the OCP after the transfer, commit/verify calls will revert (`onlyOwner`). If you point to the Timelock before the transfer, the Timelock will forward but the OCP will still reject it because the Timelock is not the owner yet.

#### 5) Verify the migration

- OCP `owner()` returns the Timelock address.
- Sequencer addresses return `true` for `hasRole(SEQUENCER, <addr>)` on the Timelock.
- The L2 node logs show commit/verify txs sent to the Timelock.

### Database migration (local node)

This migration applies to the L2 node database only (SQL-backed store). It does not change any on-chain contracts.

The `balance_diffs` table added a new `value_per_token` column of type `BLOB`:

```sql
ALTER TABLE balance_diffs
ADD COLUMN value_per_token BLOB;
```

## From v9 to v10

### CommonBridge: L2 gas limit stored on-chain

From v10 onwards, the L2 block gas limit is stored in the `CommonBridge` contract as `l2GasLimit`. The sequencer fetches this value on startup instead of using a CLI flag.

#### Upgrade requirement

On existing deployments, `l2GasLimit` will default to `0` because `initialize()` has already run. This means `_sendToL2` will revert for any non-zero gas limit, **bricking the bridge** until the owner calls `setL2GasLimit()`.

After upgrading (with the contract paused), call `setL2GasLimit` before unpausing (see [Updating the gas limit](#updating-the-gas-limit) below). The contract must not be unpaused until `l2GasLimit` is set to a valid value. Otherwise all deposits and privileged transactions will revert.

#### CLI flag removed

The `--block-producer.block-gas-limit` flag has been removed. The sequencer now reads the gas limit from the `CommonBridge` contract on startup. Update any scripts or deployment configurations that use this flag.

#### Viewing the current gas limit

```shell
rex call <BRIDGE_ADDRESS> "l2GasLimit()" --rpc-url <L1_RPC_URL>
```

#### Updating the gas limit

Only the bridge owner can update the gas limit:

```shell
rex send <BRIDGE_ADDRESS> "setL2GasLimit(uint256)" <NEW_GAS_LIMIT> \
  --private-key <OWNER_PRIVATE_KEY> \
  --rpc-url <L1_RPC_URL>
```

After updating the on-chain value, restart the sequencer for it to take effect. Until the restart, the sequencer continues using the previous gas limit, which may cause a temporary mismatch with the on-chain value.


## From v29 to v30

### SP1 6 (OnChainProposer and SP1 verifier)

The SP1 prover moves from SP1 5 to SP1 6.9.0. Its proofs need the `v6.1.0` `SP1VerifierGroth16`, and the OnChainProposer gains `upgradeSP1Verifier(address)` to point at it. Deployments that require SP1 proofs need both: the SP1 5 verifier they were deployed with rejects SP1 6 proofs whatever verification key is registered, because SP1 6 wraps its proofs in a different Groth16 circuit. The OnChainProposer has not changed since v25, so deployments from v25 onwards follow the same steps.

The OnChainProposer reads a single verifier address, so the contract steps go between two points: after every batch committed by the old version is verified, since its proofs only pass the old verifier, and before the new version submits its first proof. The v30 OnChainProposer enforces the first point: the call in step 2 reverts with `UnverifiedBatchesPending` while a batch is unverified. The nodes can move either without downtime or with a stop.

#### Contract steps

1. Deploy `SP1VerifierGroth16` from sp1-contracts `v6.1.0` and the v30 OnChainProposer implementation.
2. Through the Timelock, upgrade the OnChainProposer and point it at the new verifier in a single call, `upgradeToAndCall(<new implementation>, <calldata of upgradeSP1Verifier(<new verifier>)>)`. Use `schedule` and `execute`, or `emergencyExecute` from the Security Council. For example, with the Security Council key:

   ```shell
   UPGRADE=$(cast calldata 'upgradeToAndCall(address,bytes)' <NEW_IMPLEMENTATION> \
     $(cast calldata 'upgradeSP1Verifier(address)' <NEW_VERIFIER>))
   cast send <TIMELOCK_ADDRESS> 'emergencyExecute(address,uint256,bytes)' \
     <ON_CHAIN_PROPOSER_ADDRESS> 0 "$UPGRADE" \
     --private-key <SECURITY_COUNCIL_PK> --rpc-url <L1_RPC_URL>
   ```

3. Through the Timelock, call `upgradeSP1VerificationKey(bytes32,bytes32)` with the keccak of the v30 commit and the `ethrex-riscv32im-succinct-zkvm-vk-bn254` key from the v30 release. The v30 sequencer cannot commit a batch before its key is registered.
4. Check `SP1_VERIFIER_ADDRESS()` and `verificationKeys(<commit hash>, 1)` on the OnChainProposer.

#### Without downtime

The v30 sequencer runs next to the old one, follows it over p2p, and takes over block production at a chosen block. Only one prover version can serve a batch, and a sequencer that stops sequencing also stops sending proofs, so the old version's batches are verified before the handover:

1. Stop the old sequencer's committer with `GET /committer/stop` on its admin server. It keeps producing blocks and sending the old prover's proofs.
2. Wait until `lastVerifiedBatch` equals `lastCommittedBatch`, then stop the old prover.
3. Run the [contract steps](#contract-steps).
4. Start the v30 sequencer with its own data directory, ports and proof coordinator, and:
   - `--bootnodes <enode of the old sequencer>`, which the old sequencer logs at startup as `Local node initialized enode=...`;
   - `--admin.start-at <N>`, a block far enough ahead for it to catch up;
   - `--admin.l2-head-check-rpc-url <old sequencer RPC>`.

   Start the v30 SP1 prover against the v30 sequencer's proof coordinator.
5. Send `POST /state-updater/stop-at/<N>` to the old sequencer's admin server. It produces block `N` and stops; the v30 sequencer produces the next block.
6. Move RPC traffic to the v30 sequencer, since the old one stays at block `N`, then stop the old sequencer.

Block production does not pause. L1 verification pauses from step 1 until the v30 sequencer's first batch is proven; that batch holds the blocks the old sequencer produced after step 1.

#### With a stop

1. Stop the committer and wait until `lastVerifiedBatch` equals `lastCommittedBatch`, then stop the old sequencer and prover.
2. Run the [contract steps](#contract-steps).
3. Start the v30 sequencer and prover with the existing data directories; the node migrates its database at startup.

#### With Succinct's SP1 verifier gateway

If `SP1_VERIFIER_ADDRESS()` is Succinct's [`SP1VerifierGateway`](https://docs.succinct.xyz/docs/sp1/verification/contract-addresses), the gateway routes SP1 6 proofs to its `v6.1.0` verifier, so contract steps 1 and 2 are not needed: register the v30 key (step 3) and move the nodes. The gateway froze its `v5.0.0` route, though, so batches committed with SP1 5 and not verified before the freeze can no longer be verified. They are reverted and committed again by v30:

1. Stop the old committer and the old prover. Note `lastVerifiedBatch` as `V` and `lastCommittedBatch` as `C`; if they are equal, go to step 4.
2. Check that `batchWithdrawalLogsMerkleRoots(n)` on the CommonBridge is zero for every batch from `V + 1` to `C`. `CommonBridge` refuses to publish withdrawals twice for one batch number and `revertBatch` does not clear them, so a batch that published withdrawals cannot be committed again.
3. Through the Timelock, call `pause()`, `revertBatch(V + 1)` and `unpause()` on the OnChainProposer.
4. Register the v30 key (contract step 3).
5. Move the nodes:
   - With a stop: stop the old sequencer, run `ethrex l2 revert-batch <V> <ON_CHAIN_PROPOSER_ADDRESS> --datadir <L2 datadir>`, which keeps the batch it is given and deletes the later ones, delete the `checkpoint_batch_<n>` directories for `n` above `V` in the data directory, and start v30 on it.
   - Without downtime: hand over as above, from step 4. The v30 sequencer commits the batches the old one sealed again, generating its own prover input for each.

The [SP1 version upgrade test](../../developers/l2/sp1-upgrade-test.md) runs both.

#### Prover

The SP1 GPU prover now runs on a local `sp1-gpu-server` that the SDK downloads, so `--sp1-server` is ignored; see the [SP1 prover guide](./prover/sp1.md#troubleshooting) for GPU persistence mode and host memory.

Aligned mode is not available in v30: the Aligned SDK still verifies SP1 5 proofs.
