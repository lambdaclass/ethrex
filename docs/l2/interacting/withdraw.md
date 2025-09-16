# Withdraw assets from the L2

This section explains how to withdraw funds from the L2 through the native bridge.

## Prerequisites for L2 withdrawal

- An L2 account with sufficient ETH balance, for developing purpose you can use:
  - Address: `0x8943545177806ed17b9f23f0a21ee5948ecaa776`
  - Private Key: `0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31`
- The address of the deployed `CommonBridge` L2 contract (note here that we are calling the L2 contract instead of the L1 as in the deposit case). If not specified, You can use:
  - `CommonBridge` L2: `0x000000000000000000000000000000000000ffff`
- An Ethereum utility tool like [Rex](https://github.com/lambdaclass/rex).

## Making a withdrawal

Using Rex, we simply run the `rex l2 withdraw` command, which uses the default `CommonBridge` address.

```sh
# Format: rex l2 withdraw <AMOUNT> <PRIVATE_KEY> [RPC_URL]
rex l2 withdraw 5000 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31
```

If the withdrawal is successful, the hash will be printed like this:

```text
Withdrawal sent: <L2_WITHDRAWAL_TX_HASH>
...
```

## Claiming the withdrawal

After making a withdrawal, it has to be claimed in the L1, through the L1 `CommonBridge` contract.
For that, we can use the Rex command `rex l2 claim-withdraw`, with the tx hash obtained in the previous step.
But first, it is necessary to wait for the block that includes the withdraw to be verified.

<!-- TODO: how can we check the withdrawal was verified? -->

```sh
# Format: rex l2 claim-withdraw <L2_WITHDRAWAL_TX_HASH> <PRIVATE_KEY> <BRIDGE_ADDRESS> [L1_RPC_URL] [RPC_URL]
rex l2 claim-withdraw <L2_WITHDRAWAL_TX_HASH> 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31 0x65dd6dc5df74b7e08e92c910122f91d7b2d5184f
```

## Verifying the withdrawal

Once the withdrawal is made you can verify the balance has decreased in the L2 with:

```sh
rex l2 balance 0x8943545177806ed17b9f23f0a21ee5948ecaa776
```

And also increased in the L1:

```sh
rex balance 0x8943545177806ed17b9f23f0a21ee5948ecaa776
```
