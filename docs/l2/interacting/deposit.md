# Deposit assets into the L2

To transfer ETH from Ethereum L1 to your L2 account, you need to use the `CommonBridge` as explained in this section.

## Prerequisites for L1 deposit

- An L1 account with sufficient ETH balance, for developing purposes you can use:
  - Address: `0x8943545177806ed17b9f23f0a21ee5948ecaa776`
  - Private Key: `0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31`
- The address of the deployed `CommonBridge` contract.
- An Ethereum utility tool like [Rex](https://github.com/lambdaclass/rex)

## Making a deposit

Making a deposit in the Bridge, using Rex, is as simple as:

```sh
# Format: rex l2 deposit <AMOUNT> <PRIVATE_KEY> <BRIDGE_ADDRESS> [L1_RPC_URL]
rex l2 deposit 50000000 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31 0x65dd6dc5df74b7e08e92c910122f91d7b2d5184f
```

## Verifying the updated L2 balance

Once the deposit is made you can verify the balance has increase with:

```sh
# Format: rex l2 balance <ADDRESS> [RPC_URL]
rex l2 balance 0x8943545177806ed17b9f23f0a21ee5948ecaa776
```

For more information on what you can do with the `CommonBridge` see [Ethrex L2 contracts](../fundamentals/contracts.md).
