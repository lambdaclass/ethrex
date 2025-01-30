# Ethereum's P2P Protocol Implementation following Actors Model

> [!WARNING]
> This is a work in progress:
>
> - It is not yet ready for use.
> - This module is separate to easy testing, development, and PR preliminary reviews.

## Acknowledgements

We wanted to thank and acknowledge [Commonware](https://commonware.xyz/). Most of this code was inspired by their [`monorepo`](https://github.com/commonwarexyz/monorepo/).

## Testing Discovery

Run the following and read the logs. CTRL-C to stop.

```
cargo run --release --example discovery
```

## Testing RLPx as Initiator

Run the following and read the logs. CTRL-C to stop.

```
cargo run --release --example rlpx_initiator
```

## Testing RLPx as Receiver

Run the following and read the logs. CTRL-C to stop.

```
cargo run --release --example rlpx_initiator
```
