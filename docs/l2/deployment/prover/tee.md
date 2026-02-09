# Run an ethrex TDX prover

In this section, we'll guide you through the steps to run an ethrex L2 TDX prover for generating TEE proofs. These proofs are essential for validating batch execution and state settlement on your ethrex L2.

## Prerequisites

- This guide assumes that you have already deployed an ethrex L2 with TDX enabled. If you haven't done so yet, please refer to one of the [Deploying an ethrex L2](../overview.md) guides.
- A machine with TDX support [with the required setup](https://github.com/canonical/tdx).

## Start an ethrex L2 TDX prover

There's no official release of our ethrex L2 TDX prover yet, so you need to build ethrex from source. To do this, clone the ethrex repository and run:

```shell
git clone https://github.com/lambdaclass/ethrex.git

cd ethrex/crates/l2/tee/quote-gen

make run
```

> [!NOTE]
> Refer to the [TDX guide](../../architecture/tdx.md) for more information on setting up and running the quote generator.
