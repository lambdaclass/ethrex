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
> Cualquiera haya sido el metodo de instalacion de ethrex, asegurate de que el binario que estas utilizando tiene soporte para SP1, y tambien para GPU si es que tu intencion es correr un prover SP1 GPU.

> [!NOTE]
> The flag `--proof-coordinators` is used to specify one or more proof coordinator URLs. This is so because the prover is capable of proving ethrex L2 batches from multiple sequencers. We are particularly setting it to `localhost:3900` because the command above command uses the port `3900` for the proof coordinator by default (to learn more about the proof coordinator, read the ethrex L2 sequencer and ethrex L2 prover sections).
> We choose SP1 as the backend to indicate the prover to generate SP1 proofs.

## Troubleshooting

### `docker: Error response from daemon: could not select device driver "" with capabilities: [[gpu]]`

If you encounter the following error when starting the SP1 prover with GPU support:

```plaintext
docker: Error response from daemon: could not select device driver "" with capabilities: [[gpu]]
```

This error indicates that Docker is unable to find a suitable GPU driver for running containers with GPU support. To resolve this issue, follow these steps:

1. **Install NVIDIA Container Toolkit**: Ensure that you have the NVIDIA Container Toolkit installed on your system. This toolkit allows Docker to utilize NVIDIA GPUs. You can follow the installation instructions from the [official NVIDIA documentation](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/install-guide.html).
2. **Configure Docker to use the NVIDIA runtime**: After installing the NVIDIA Container Toolkit, you need to configure Docker to use the NVIDIA runtime by default. You can do this by following the instructions in the [Configuring Docker documentation](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/install-guide.html#configuring-docker).
