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
