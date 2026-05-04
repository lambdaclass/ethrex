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

## From v10 to v11

### CommonBridge: L2 gas limit stored on-chain

From v11 onwards, the L2 block gas limit is stored in the `CommonBridge` contract as `l2GasLimit`. The sequencer fetches this value on startup instead of using a CLI flag.

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
