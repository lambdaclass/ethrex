# Deploy a Contract to L2

You can deploy smart contracts to your L2 using [`rex`](https://github.com/lambdaclass/rex), a simple CLI tool for interacting with Ethereum-compatible networks.

## 1. Generate the Contract Bytecode

First, compile your Solidity contract to get the deployment bytecode. You can use [solc (v0.8.31)](https://docs.soliditylang.org/en/latest/installing-solidity.html) for this:

```sh
solc --bin MyContract.sol -o out/
```

The bytecode will be in out/MyContract.bin

## 2. Deploy with rex

Use the following command to deploy your contract:

```sh
rex deploy --rpc-url http://localhost:1729 <BYTECODE> 0 <PRIVATE_KEY>
```

- Replace `<BYTECODE>` with the hex string from your compiled contract (e.g., contents of `MyContract.bin`)
- Replace `<PRIVATE_KEY>` with your wallet's private key. It must have funds in L2
- Adjust the `--rpc-url` if your L2 node is running elsewhere

For more details and advanced usage, see the [rex repository](https://github.com/lambdaclass/rex).
