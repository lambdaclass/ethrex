# Ethrex L2 contracts

There are two L1 contracts: OnChainProposer and CommonBridge. Both contracts are deployed using UUPS proxies, so they are upgradeables.

### Upgrade the contracts

To upgrade a contract, you have to create the new contract and, as the original one, inherit from OpenZeppelin's `UUPSUpgradeable`. Make sure to implement the `_authorizeUpgrade` function and follow the [proxy pattern restrictions](https://docs.openzeppelin.com/upgrades-plugins/writing-upgradeable).

Once you have the new contract, you need to do the following two steps:

- Deploy the new contract
- Upgrade the proxy by calling the method `upgradeToAndCall(address newImplementation, bytes memory data)`. The `data` parameter is the calldata to call on the new implementation as an initialization, you can pass an empty stream.
