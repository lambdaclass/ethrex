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

## Configuration

The program that deploys our L2 contracts outputs the addresses in a `.env` file, that includes environment information used by each component, automatically loaded by our makefile.
Apart from these, each component accepts multiple configuration options, which can be configured either in the `.env`, or with CLI flags.
More information is available in [the documentation for each component](./components.md).

## Guides

For more information on how to perform certain operations, go to [Guides](./guides).
