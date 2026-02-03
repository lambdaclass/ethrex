# Exit Window

An exit window is a time window, or period, during which users can opt to exit the network before the execution of an upgrade or system modification. The purpose of exit windows in L2 rollups is to protect users from unwanted changes to the system, such as those mentioned above.

The [Stages Framework](https://forum.l2beat.com/t/the-stages-framework/291) defines exit windows for rollup upgrades with subtle differences between stages. For Stage 1 rollups, updates initiated outside the Security Council require an exit window of at least 7 days, though the Security Council can upgrade instantly. For Stage 2 rollups, the Security Council can upgrade immediately only if a bug is detected on-chain; otherwise, the exit window should be at least 30 days. This period may vary if there is a withdrawal delay, as it is subtracted from the total exit window.

The ethrex L2 stack provides this security functionality through a `Timelock` contract that is deployed and configured with the exit window duration, which we will learn more about in the next section.

## How exit windows work

Before understanding how exit windows work, it is necessary to keep in mind which specific functionality of the L1 contracts we need to protect. For this, we recommend reading in advance about the `OnChainProposer` and `CommonBridge` contracts in the [contracts fundamentals section](./contracts.md). To make it simpler, we will initially focus only on the upgrade logic, as the same logic applies to the rest of the modifications.

All our contracts are [`UUPSUpgradeable`](https://docs.openzeppelin.com/contracts/5.x/api/proxy#UUPSUpgradeable) (an upgradeability pattern recommended by OpenZeppelin). In particular, to upgrade this type of contract, the operator must call the `upgradeToAndCall` function, which invokes an internal function called `_authorizeUpgrade`. It is recommended to override this function by implementing authorization logic. This is the function we must protect in the case of both contracts, and we do so by “delaying” its execution.

Currently, the function used to upgrade the contracts is protected by an `onlyOwner` modifier, which verifies that the caller corresponds to the owner of the contract (all L1 contracts but the `Timelock` are [`Ownable2StepUpgradeable`](https://docs.openzeppelin.com/contracts/5.x/api/access#ownable2step)), configured during its initialization. In other words, only the owner can call it. Keeping this in mind is important for understanding how we implement the functionality.

As mentioned earlier, exit windows must prevent the instantaneous execution of upgrades and modifications to the system (from now on, operations). This is achieved through the aforementioned `Timelock` contract, which introduces a notion of “delay” to the execution of operations.

To accomplish this, the `Timelock` contract divides the execution of operations into two steps:

1. A first step where the operation is scheduled.
2. A second step that finally executes the previously scheduled operation.

In the scheduling step, the information corresponding to the operation (calldata, target address, value, etc.) is stored in the contract’s storage along with the timestamp from which the operation can be executed (essentially the current timestamp at the time of scheduling plus the delay configured during the contract’s initialization). The caller is also granted an operation ID.

It is in the second and final step where the previously scheduled operation is executed. This occurs if and only if the waiting time has been met (i.e., the timestamp at the time of executing the operation is greater than or equal to the operation’s timestamp stored in the contract’s storage). It is worth noting that any attempt to execute prior to the fulfillment of the waiting time will revert. The contract offers additional functionality to check the status of operations and thus avoid reverting execution attempts.

We achieve an exit window by configuring the `Timelock` contract as the owner of the L1 contracts. In this way, it is the only one capable of executing upgrades on the L1 contracts, and it will do so through the scheduling and execution of operations, which provide the desired delay. With this, it is sufficient to add the onlyOwner modifier to the functions we want to execute with a certain delay.

## Settlement window

Also known as “withdrawal delay”, the settlement window is the batch verification delay that needs to be fulfilled for the sequencer to be able to verify a committed batch, even if the proof is already available for verification.

The goal of the settlement window is to give enough time to the rollup operator to react in the event of a bug exploit in the L2 before the state is settled on the L1 and, thus, irreversible.

As said before, the settlement window must be taken into account to calculate the real exit window.

## Who owns the `Timelock`

There's no such thing as an unique owner of the `Timelock` necessarily. `Timelock` is a `TimelockController`, which is also an `AccessControl`, so we can define different roles and assign them to different entities. By "owner" of the `Timelock`, we refer to the account that has the role to update the contract (i.e. the one that can modify the delay).

That said, whoever owns the `Timelock` decides its functioning. In our stack, the owner of the contract is established during its initialization, and then that owner can transfer the ownership to another account if desired.

It's worth noting that the designated security council can execute operations instantly in case of emergencies, so it's crucial that the members are trustworthy and committed to the network's security. The [Stages Framework](https://forum.l2beat.com/t/the-stages-framework/291) recommends that the security council be in the form of a multisig composed of at least 8 people with a consensus threshold of 75%, and the participants in the set need to be decentralized and diverse enough, possibly coming from different companies and jurisdictions.

In the case of our `Timelock`, the owner is not the only one who can act on it. In fact, it is recommended that the security council only act in specific emergencies. The `Timelock` is also `AccessControl`, which means it has special functionality for managing accesses, in this case, in the form of roles.

`TimelockController` defines two roles in its business logic: the “proposer” and the “executor.” The first is enabled to schedule operations and cancel them, while the second is enabled to execute them. These roles are assigned to a given set of addresses during the contract’s initialization.

We define two additional roles besides the defaults: the “sequencer” and the “security council.” The first is confined to performing settlement operations (meaning it cannot operate as “proposer” or “executor”), while the second is enabled to revert batches, pause and unpause contracts, and execute emergency operations (i.e., without delay).

## How are `Timelock` upgrades protected

Today, we only expose two types of modifications to our `Timelock` contract:

1. Upgrade of the contract itself.
2. Update of the delay time.

An interesting question is how we protect users from instantaneous executions of these types of operations. One might think of a scenario where the `Timelock` owner updates the delay to 0 to execute a malicious operation, having removed the exit window for users; or perhaps upgrade the `Timelock` contract by removing the delay logic with the same objective.

We solve this by making the `Timelock` itself the only one capable of invoking the corresponding functions for those operations. In this way, if the contract owner wants to push a malicious upgrade or modification to the “protector” contract, they must comply with the configured delay at the time of proposing the operation. For example, if a malicious or compromised owner wishes to set the delay to 0, they must wait the duration of the current exit window for the execution of their malicious operation, thus giving users sufficient time to exit the network.
