# ethrex L2 Docs

For a high level overview of the L2:

- [General Overview](./overview.md)

For more detailed documentation on each part of the system:

- [Sequencer](./sequencer.md): Describes the components and configuration of the L2 sequencer node.
- [Contracts](./contracts.md): Explains the L1 and L2 smart contracts used by the system.
- [Prover](./prover.md): Details how block execution proofs are generated and verified using zkVMs.
- [State Diffs](./state_diffs.md): Specifies the format for state changes published for data availability.
- [Withdrawals](./withdrawals.md): Explains the mechanism for withdrawing funds from L2 back to L1.

- [Rust](https://www.rust-lang.org/tools/install)
- [Solc 0.29](https://docs.soliditylang.org/en/latest/installing-solidity.html)
- [Docker](https://docs.docker.com/engine/install/)
  
## Quick HandsOn

Make sure docker is running!

1. `cd crates/l2`
2. `make rm-db-l2 && make down`
   - It will remove any old database, if present, stored in your computer. The absolute path of libmdbx is defined by [data_dir](https://docs.rs/dirs/latest/dirs/fn.data_dir.html).
4. `make init`
   - Init the L1 in a docker container on port `8545`.
   - Deploy the needed contracts for the L2 on the L1.
   - Start the L2 locally on port `1729`.


For more information on how to run the L2 node with the prover attached to it, the [Prover Docs](./prover.md) provides more insight.

## Bridge Assets

### Funding an L2 Account from L1

To transfer ETH from Ethereum L1 to your L2 account:

1. Prerequisites:
   - An L1 account with sufficient ETH balance, for developing purpose you can use:
      - Address: `0x8943545177806ed17b9f23f0a21ee5948ecaa776`
      - Private Key: `0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31`
   - The address of the deployed CommonBridge contract. 
   - An Ethereum utility tool like [Rex](https://github.com/lambdaclass/rex)

2. Make a deposit:

   Using Rex is as simple as:
   ```Shell
   # Format: rex l2 deposit <AMOUNT> <PRIVATE_KEY> <BRIDGE_ADDRESS> [L2_RPC_URL]
   rex l2 deposit 50000000 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31 0x65dd6dc5df74b7e08e92c910122f91d7b2d5184f
   ```

3. Verification:

   Once the deposit is made you can verify the balance has increase with:
   ```Shell
   # Format: rex l2 balance <Address> [RPC_URL]
   rex l2 balance 0x8943545177806ed17b9f23f0a21ee5948ecaa776
   ```

For more information on what you can do with the CommonBridge see [here](./contracts.md).

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

## Configuration

Configuration consists of the creation and modification of a `.env` file done automatically by the contract deployer, then each component reads the `.env` to load the environment variables. A detailed list is available in each part documentation.

## Testing

Load tests are available via L2 CLI and Makefile targets.

### Makefile

There are currently three different load tests you can run:

```
make load-test
make load-test-fibonacci
make load-test-io
```

The first one sends regular transfers between accounts, the second runs an EVM-heavy contract that computes fibonacci numbers, the third a heavy IO contract that writes to 100 storage slots per transaction.

## Load test comparison against Reth

To run a load test on Reth, clone the repo, then run

```
cargo run --release -- node --chain <path_to_genesis-load-test.json> --dev --dev.block-time 5000ms --http.port 1729
```

to spin up a reth node in `dev` mode that will produce a block every 5 seconds.

Reth has a default mempool size of 10k transactions. If the load test goes too fast it will reach the limit; if you want to increase mempool limits pass the following flags:

```
--txpool.max-pending-txns 100000000 --txpool.max-new-txns 1000000000 --txpool.pending-max-count 100000000 --txpool.pending-max-size 10000000000 --txpool.basefee-max-count 100000000000 --txpool.basefee-max-size 1000000000000 --txpool.queued-max-count 1000000000
```

### Changing block gas limit

By default the block gas limit is the one Ethereum mainnet uses, i.e. 30 million gas. If you wish to change it, just edit the `gasLimit` field in the genesis file (in the case of `ethrex` it's `l2.json`, in the case of `reth` it's `load-test.json`). Note that the number has to be passed as a hextstring.

## Flamegraphs

To analyze performance during load tests (both `ethrex` and `reth`) you can use `cargo flamegraph` to generate a flamegraph of the node.

For `ethrex`, you can run the server with:

```
sudo -E CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph --bin ethrex --features dev  --  --network test_data/fixtures/genesis/l2.json --http.port 1729 --dev
```

For `reth`:

```
sudo cargo flamegraph --profile profiling -- node --chain <path_to_genesis-load-test.json> --dev --dev.block-time 5000ms --http.port 1729
```

### With Make Targets

There are some make targets inside the root's Makefile.

You will need two terminals:
1. `make start-node-with-flamegraph` &rarr; This starts the ethrex client.
2. `make flamegraph` &rarr; This starts a script that sends a bunch of transactions, the script will stop ethrex when the account reaches a certain balance.

### Samply

To run with samply, run

```
samply record ./target/profiling/reth node --chain ../ethrex/test_data/fixtures/genesis/load-test.json --dev --dev.block-time 5000ms --http.port 1729
```
