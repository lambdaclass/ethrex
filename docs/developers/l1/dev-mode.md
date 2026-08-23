# Ethrex as a local development node

## Prerequisites

This guide assumes you've read the dev [installation guide](../installing.md)

## Dev mode

In dev mode ethrex acts as a local Ethereum development node. It can be run with the following command

```sh
ethrex --dev
```

Then you can use a tool like [rex](https://github.com/lambdaclass/rex) to make sure that the network is advancing

```sh
rex block-number
```

Rich account private keys are listed at the folder `fixtures/keys/private_keys_l1.txt` located at the root of the repo. You can then use these keys to deploy contracts and send transactions in the localnet.

## Amsterdam and Hegotá

The default dev genesis stops at Osaka. To exercise Amsterdam or Hegotá features
(EIP-8141 frame transactions among them), point `--network` at the Hegotá dev genesis:

```sh
ethrex --dev --network fixtures/genesis/l1-hegota.json
```

It carries the same rich accounts as `l1.json` plus the two EIP-8282 builder
deposit/exit predeploys. Those are part of Glamsterdam (EIP-7773) and, like the
EIP-7002/EIP-7251 predeploys, they come from genesis rather than being installed by
the client; on an Amsterdam+ block their empty code invalidates the block, so a
genesis without them cannot produce one:

```
ERROR Failed to produce block: System contract: 0x0000…8282 has no code after deployment
ERROR block producer failed: EngineClient System Failed after: 3; shutting down the node
```

The Hegotá predeploys (`0x…8141`, `0x…8250`) need no genesis entry: the client
installs them at the fork boundary.

