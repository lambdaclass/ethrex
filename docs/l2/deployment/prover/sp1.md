# Run an ethrex L2 SP1 prover

In this section, we'll guide you through the steps to run an ethrex L2 prover that utilizes SP1 for generating ZK proofs. These proofs are essential for validating batch execution and state settlement on your ethrex L2.

## Prerequisites

- This guide assumes that you have ethrex installed with the SP1 feature and available in your PATH. If you haven't installed it yet, follow one of the methods in the Installation Guide. If you want to build the binary from source, refer to the [Building from source](./overview.md#building-from-source-skip-if-ethrex-is-already-installed) section and select the appropriate build option.
- This guide also assumes that you have already deployed an ethrex L2 with SP1 enabled. If you haven't done so yet, please refer to one of the [Deploying an ethrex L2](../overview.md) guides.

## Start an ethrex L2 SP1 prover

Once you have your ethrex L2 deployed with SP1 enabled, you can start the SP1 prover using the following command:

```shell
ethrex l2 prover \
--backend sp1 \
--proof-coordinators http://localhost:3900
```

> [!IMPORTANT]
> Regardless of the installation method used for ethrex, make sure the binary you are using has SP1 support, and also GPU support if you intend to run an SP1 GPU prover.

> [!NOTE]
> The flag `--proof-coordinators` is used to specify one or more proof coordinator URLs. This is so because the prover is capable of proving ethrex L2 batches from multiple sequencers. We are particularly setting it to `localhost:3900` because the command above uses the port `3900` for the proof coordinator by default (to learn more about the proof coordinator, read the ethrex L2 sequencer and ethrex L2 prover sections).
> We choose SP1 as the backend to indicate the prover to generate SP1 proofs.

## Troubleshooting

### `Could not connect to sp1-gpu-server socket`

If the SP1 GPU prover stops at startup with:

```plaintext
Failed to create the CUDA prover impl: CudaClientError: Connect(Os { code: 2, kind: NotFound, message: "No such file or directory" })
Context: "Could not connect to `sp1-gpu-server` socket"
```

the SP1 SDK started its `sp1-gpu-server`, but the server did not open its socket within the one second the SDK waits for it. The server initializes the GPU before it opens the socket, and with persistence mode disabled the NVIDIA driver initializes the GPU again on every start, which can take a few seconds. Enable persistence mode and start the prover again:

```shell
sudo nvidia-smi -pm 1
```

Persistence mode is reset on reboot. To keep it enabled, run the `nvidia-persistenced` service.
