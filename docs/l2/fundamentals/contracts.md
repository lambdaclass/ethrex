# Ethrex L2 contracts

There are two L1 contracts: OnChainProposer and CommonBridge. Both contracts are deployed using UUPS proxies, so they are upgradeables.

## L1 side

### `CommonBridge`

Allows L1<->L2 communication from L1. It both sends messages from L1 to L2 and receives messages from L2.

#### Deposit Functions

##### Simple Deposits

- Send ETH directly to the contract address using a standard transfer
- The contract's `receive()` function automatically forwards funds to your identical address on L2
- No additional parameters needed

##### Deposits with Contract Interaction

```solidity
function deposit(DepositValues calldata depositValues) public payable
```

Parameters:

- `to`: Target address on L2
- `recipient`: Address that will receive the ETH on L2 (can differ from sender)
- `gasLimit`: Maximum gas for L2 execution
- `data`: Calldata to execute on the target L2 contract

This method enables atomic operations like:

- Depositing ETH while simultaneously interacting with L2 contracts
- Funding another user's L2 account

### `OnChainOperator`

Ensures the advancement of the L2. It is used by the operator to commit batches of blocks and verify batch proofs.

### `Verifier`

TODO

## L2 side

### `L1MessageSender`

TODO

## Upgrade the contracts

To upgrade a contract, you have to create the new contract and, as the original one, inherit from OpenZeppelin's `UUPSUpgradeable`. Make sure to implement the `_authorizeUpgrade` function and follow the [proxy pattern restrictions](https://docs.openzeppelin.com/upgrades-plugins/writing-upgradeable).

Once you have the new contract, you need to do the following three steps:

1. Deploy the new contract

    ```sh
    rex deploy <NEW_IMPLEMENTATION_BYTECODE> 0 <DEPLOYER_PRIVATE_KEY>
    ```

2. Upgrade the proxy by calling the method `upgradeToAndCall(address newImplementation, bytes memory data)`. The `data` parameter is the calldata to call on the new implementation as an initialization, you can pass an empty stream.

    ```sh
    rex send <PROXY_ADDRESS> 'upgradeToAndCall(address,bytes)' <NEW_IMPLEMENTATION_ADDRESS> <INITIALIZATION_CALLDATA> --private-key <PRIVATE_KEY>
    ```

3. Check the proxy updated the pointed address to the new implementation. It should return the address of the new implementation:

    ```sh
    curl http://localhost:8545 -d '{"jsonrpc": "2.0", "id": "1", "method": "eth_getStorageAt", "params": [<PROXY_ADDRESS>, "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc", "latest"]}'
    ```

## Transfer ownership

The contracts are `Ownable2Step`, that means that whenever you want to transfer the ownership, the new owner have to accept it to effectively apply the change. This is an extra step of security, to avoid accidentally transfer ownership to a wrong account. You can make the transfer in these steps:

1. Start the transfer:

    ```sh
    rex send <PROXY_ADDRESS> 'transferOwnership(address)' <NEW_OWNER_ADDRESS> --private-key <CURRENT_OWNER_PRIVATE_KEY>
    ```

2. Accept the ownership:

    ```sh
    rex send <PROXY_ADDRESS> 'acceptOwnership()' --private-key <NEW_OWNER_PRIVATE_KEY>
    ```
