# Run multiple provers

In this section, we'll guide you through the steps to run multiple ethrex L2 provers for generating ZK proofs using different backends. These proofs are essential for validating batch execution and state settlement on your ethrex L2.

## Prerequisites

- This guide assumes that you have already deployed an ethrex L2 with TDX enabled. If you haven't done so yet, please refer to one of the [Deploying an ethrex L2](../overview.md) guides.

## Start multiple ethrex L2 provers

Once you have your ethrex L2 deployed with multiple proving backends enabled (SP1, RISC0, TDX), refer to the following guides to start each prover:

- [Run an ethrex L2 SP1 prover](./sp1.md)
- [Run an ethrex L2 RISC0 prover](./risc0.md)
- [Run an ethrex TDX prover](./tee.md)

Each prover should be started in different machines to ensure they operate independently and efficiently. Make sure to configure each prover with the appropriate backend flag and proof coordinator URLs as specified in their respective guides.
