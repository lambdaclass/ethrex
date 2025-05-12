# Ethrex L2 contracts

There are two L1 contracts: OnChainProposer and CommonBridge. Both contracts are deployed using UUPS proxies, so they are upgradeables.

### Upgrade the contracts

To upgrade a contract, you have to create the new contract and, as the original one, inherit from OpenZeppelin's `UUPSUpgradeable`. Make sure to implement the `_authorizeUpgrade` function and follow the [proxy pattern restrictions](https://docs.openzeppelin.com/upgrades-plugins/writing-upgradeable).

Once you have the new contract, you need to do the following two steps:

1. Deploy the new contract
  ```sh
  rex deploy <NEW_IMPLEMENTATION_BYTECODE> 0 <DEPLOYER_PRIVATE_KEY>
  ```
2. Upgrade the proxy by calling the method `upgradeToAndCall(address newImplementation, bytes memory data)`. The `data` parameter is the calldata to call on the new implementation as an initialization, you can pass an empty stream.
  ```sh
  rex send <PROXY_ADDRESS> 0 <PRIVATE_KEY> -- 'upgradeToAndCall(address,bytes)' 0 <NEW_IMPLEMENTATION_ADDRESS> <INITIALIZATION_CALLDATA>
  ```
3. Check the proxy updated the pointed address to the new implementation. It should return the address of the new implementation:
  ```sh
  curl http://localhost:8545 -d '{"jsonrpc": "2.0", "id": "1", "method": "eth_getStorageAt", "params": [<PROXY_ADDRESS>, "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc", "latest"]}'
  ```
