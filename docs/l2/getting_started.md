# Getting started with ethrex L2 stack

## Starting the L2

> [!IMPORTANT]
> Make sure docker is running!

1. `cd crates/l2`
2. `make rm-db-l2 && make down`
   - This will remove any old database stored in your computer, if present. The absolute path of libmdbx is defined by [data_dir](https://docs.rs/dirs/latest/dirs/fn.data_dir.html).
3. `make init`
   - Starts the L1 in a docker container on port `8545`.
   - Deploys the needed contracts for the L2 on the L1.
   - Starts the L2 locally on port `1729`.

For more information on how to run the L2 node with the prover attached to it, the [Prover Docs](./prover.md) provides more insight.

## Bridge Assets

### Funding an L2 Account from L1

To transfer ETH from Ethereum L1 to your L2 account, you need to use the `CommonBridge` as explained in this section.

#### Prerequisites

- An L1 account with sufficient ETH balance, for developing purposes you can use:
  - Address: `0x8943545177806ed17b9f23f0a21ee5948ecaa776`
  - Private Key: `0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31`
- The address of the deployed `CommonBridge` contract.
- An Ethereum utility tool like [Rex](https://github.com/lambdaclass/rex)

#### Making a deposit

Making a deposit in the Bridge, using Rex, is as simple as:

```sh
# Format: rex l2 deposit <AMOUNT> <PRIVATE_KEY> <BRIDGE_ADDRESS> [L1_RPC_URL]
rex l2 deposit 50000000 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31 0x65dd6dc5df74b7e08e92c910122f91d7b2d5184f
```

#### Verifying the updated L2 balance

Once the deposit is made you can verify the balance has increase with:

```sh
# Format: rex l2 balance <ADDRESS> [RPC_URL]
rex l2 balance 0x8943545177806ed17b9f23f0a21ee5948ecaa776
```

For more information on what you can do with the CommonBridge see [Ethrex L2 contracts](./contracts.md).

### Withdrawing funds from the L2 to L1

1. Prerequisites:
   - An L2 account with sufficient ETH balance, for developing purpose you can use:
      - Address: `0x8943545177806ed17b9f23f0a21ee5948ecaa776`
      - Private Key: `0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31`
   - The address of the deployed CommonBridge L2 contract (note here that we are calling the L2 contract instead of the L1 as in the deposit case). You can use:
      - CommonBridge L2: `0x000000000000000000000000000000000000ffff`
   - An Ethereum utility tool like [Rex](https://github.com/lambdaclass/rex).

2. Make the Withdraw:

    Using Rex we simply use the `rex l2 withdraw` command (it uses the default CommonBridge address).
    ```Shell
    # Format: rex l2 withdraw <AMOUNT> <PRIVATE_KEY> [RPC_URL]
    rex l2 withdraw 5000 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31
    ```

    If the withdraw is successful, the hash will be printed in the format:

    ```
    Withdrawal sent: <L2_WITHDRAWAL_TX_HASH>
    ...
    ```

3. Claim the Withdraw:

   After making the withdraw it has to be claimed in the L1. This is done with the L1 CommonBridge contract. We can use the Rex command `rex l2 claim-withdraw`. Here we have to use the tx hash obtained in the previous step. Also, it is necessary to wait for the block that includes the withdraw to be verified.

   ```Shell
   # Format: rex l2 claim-withdraw <L2_WITHDRAWAL_TX_HASH> <PRIVATE_KEY> <BRIDGE_ADDRESS>
   rex l2 claim-withdraw <L2_WITHDRAWAL_TX_HASH> 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31 0x65dd6dc5df74b7e08e92c910122f91d7b2d5184f
   ```

4. Verification:

   Once the withdrawal is made you can verify the balance has decrease with:
   ```Shell
   rex l2 balance 0x8943545177806ed17b9f23f0a21ee5948ecaa776
   ```

   And also increased in the L1:
   ```Shell
   rex balance 0x8943545177806ed17b9f23f0a21ee5948ecaa776
   ```
