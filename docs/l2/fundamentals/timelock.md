# Timelock Contract

The Timelock contract gates access to the OnChainProposer (OCP) contract. Changes to the OCP can only be made by first interacting with the Timelock, which manages permissions based on roles assigned to different users.

## Timelock Roles

- Sequencers: Can commit and verify batches.
- Governance: Can schedule and execute operations, respecting a delay. In practice this could be the role of a DAO, though it depends on the implementation.
- Security Council: Can bypass the minimum delay for executing any operation that the Timelock can execute. It can also manage other roles in the Timelock.

**Sequencers** will send `commitBatch`, `verifyBatches`, and `verifyBatchesAligned` to the Timelock, and this will execute the operations in the `OnChainProposer`. Eventually there will be Timelock logic, and there will be a time window between commitment and proof verification for security reasons.

The **Governance** is able to schedule important operations like contract upgrades respecting the minimum time window for the L2 participants to exit in case of undesired updates. Not only can they make changes in the logic of the OnChainProposer, but they can also update the Timelock itself.

The **Security Council** is designed as a powerful entity that can execute anything within the Timelock or OnChainProposer without delay. We call it security council because its actions are limitless, as it can upgrade any of the contracts whenever it wants, so ideally it should be a multisig composed of many diverse members, and it should be able to take action only if 75% of members agree. Ideally, in a more mature rollup the Security Council would have fewer permissions and would only need to act upon bugs detected on-chain if such a mechanism exists.
We call this mechanism of executing without delay the `emergencyExecute`.


## Basic Functionalities

These are the things that we can do with the Timelock:
- Schedule: `schedule(...)` and `scheduleBatch(...)`
- Execute: `execute(...)` and `executeBatch(...)`
- Cancel: `cancel(bytes32 id)`
- Update Delay: `updateDelay(uint256 newDelay)`

When an operation is **scheduled**, the Governance role may **cancel** it or, after the established delay, **execute** it.
The delay can be updated, always respecting the current delay to do so.

It also has a few utility functions:
- `getMinDelay()`: current minimum delay for new schedules.
- `hashOperation(...)`, `hashOperationBatch(...)`: pure helpers to compute ids.
- `getTimestamp(id)`, `getOperationState(id)`, `isOperation*`: query operation status.

Remember that `Timelock` inherits from `TimelockControllerUpgradeable` (which itself extends `AccessControlUpgradeable`) and `UUPSUpgradeable`, so it will inherit their behavior as well.

## Important Remarks

### Operation ID collision

Every scheduled operation is identified by a 32-byte **operation id**. This ID is determined by hashing fields like the target address, value transferred, data, predecessor, and salt.
Two operations with the same fields will result in the same ID. That's why, if we want to schedule the same operation more than once, we should probably use a salt.
Example: If for some reason we want to schedule the pause of the OnChainProposer and we use salt zero, the next time we schedule that same operation we'll have to change the salt (assuming no predecessor was specified) in order for the id to be different.

### Cancelling a scheduled operation

`cancel(bytes32 id)` requires the operation id. You typically get it by:

1. Reading it from the `CallScheduled(id, ...)` event emitted by `schedule`/`scheduleBatch`, or
2. Computing it yourself (off-chain), or
3. Calling `hashOperation(...)` / `hashOperationBatch(...)` on-chain to compute it.

Note that:
- `hashOperation(...) = keccak256(abi.encode(target, value, data, predecessor, salt))`
- `hashOperationBatch(...) = keccak256(abi.encode(targets, values, payloads, predecessor, salt))`
