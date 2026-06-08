# Run an ethrex L2 RISC0 prover

In this section, we'll guide you through the steps to run an ethrex L2 prover that utilizes RISC0 for generating ZK proofs. These proofs are essential for validating batch execution and state settlement on your ethrex L2.

## Prerequisites

- This guide assumes that you have ethrex installed with the RISC0 feature and available in your PATH. If you haven't installed it yet, follow one of the methods in the Installation Guide. If you want to build the binary from source, refer to the [Building from source](./overview.md#building-from-source-skip-if-ethrex-is-already-installed) section and select the appropriate build option.
- This guide also assumes that you have already deployed an ethrex L2 with RISC0 enabled. If you haven't done so yet, please refer to one of the [Deploying an ethrex L2](../overview.md) guides.

## Start an ethrex L2 RISC0 prover

Once you have your ethrex L2 deployed with RISC0 enabled, you can start the RISC0 prover using the following command:

```shell
ethrex l2 prover \
--backend risc0 \
--proof-coordinators http://localhost:3900
```

> [!IMPORTANT]
> Regardless of the installation method used for ethrex, make sure the binary you are using has RISC0 support, and also GPU support if you intend to run a RISC0 GPU prover.

> [!NOTE]
> The flag `--proof-coordinators` is used to specify one or more proof coordinator URLs. This is so because the prover is capable of proving ethrex L2 batches from multiple sequencers. We are particularly setting it to `localhost:3900` because the command above uses the port `3900` for the proof coordinator by default (to learn more about the proof coordinator, read the ethrex L2 sequencer and ethrex L2 prover sections).
> We choose RISC0 as the backend to indicate the prover to generate RISC0 proofs.
