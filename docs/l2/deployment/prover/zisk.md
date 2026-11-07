# Run an ethrex L2 ZisK prover

In this section, we'll guide you through the steps to run an ethrex L2 prover that utilizes [ZisK](https://github.com/0xPolygonHermez/zisk) for generating ZK proofs. These proofs are essential for validating batch execution and state settlement on your ethrex L2.

## Prerequisites

- A Linux x86-64 host. The ZisK prover library only builds there, so the `zisk` feature of ethrex does too. Proving on GPU needs an NVIDIA driver and CUDA Toolkit 12.9 or later installed before building ethrex, since the prover library compiles its CUDA kernels when it finds `nvcc` (either in `PATH` or at `/usr/local/cuda/bin/nvcc`). ZisK v1.3.2-alpha plans one 19.5 GB proving stream plus the fixed polynomials on the device, which does not fit a 24 GB card (an RTX 3090 fails with `Not enough GPU memory to run the proof`, with `cargo-zisk prove --gpu` as much as with ethrex), so GPU proving needs a larger card such as a 48 GB A6000; on smaller cards build without the `gpu` feature and prove on CPU.
- On hosts with glibc 2.41 or newer (Debian 13, Ubuntu 25.10), start the prover with `GLIBC_TUNABLES=glibc.rtld.execstack=2`. ZisK's PLONK setup loads `provingKeySnark/final/final.so`, a shared object that asks for an executable stack, and newer loaders refuse it otherwise (`cannot enable executable stack as shared object requires`). `make init-prover-zisk` sets it.
- The ZisK bundle of the release ethrex is pinned to (v1.3.2-alpha), installed with `ziskup`. It provides the `zisk` Rust toolchain that compiles the guest program, the `cargo-zisk` CLI, the STARK proving key and the PLONK proving key that makes proofs verifiable on-chain. Follow [ZisK's Linux installation guide](https://0xpolygonhermez.github.io/zisk-docs/developer/getting-started/install-linux) for the system packages, then:

    ```shell
    curl -fsSL https://raw.githubusercontent.com/0xPolygonHermez/zisk/v1.3.2-alpha/ziskup/ziskup -o ziskup && chmod +x ziskup
    ./ziskup -v 1.3.2-alpha --gpu --provingkey -y   # drop --gpu for a CPU-only prover
    ~/.zisk/bin/cargo-zisk toolchain install -t zisk-4.0.0
    ~/.zisk/bin/ziskup setup_snark                   # the PLONK proving key, about 25 GB
    export PATH="$HOME/.zisk/bin:$PATH"
    ```

    The STARK proving key with its constant trees takes around 70 GB on disk and the PLONK proving key another 25 GB. Wrapping a proof into PLONK loads the PLONK key into memory, so the host needs that much RAM on top of what STARK proving uses.

- ethrex installed with the `zisk` feature (and `gpu` for GPU proving) and available in your `PATH`. Build it from source as described in the [Building from source](./overview.md#building-from-source-skip-if-ethrex-is-already-installed) section:

    ```shell
    COMPILE_CONTRACTS=true cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,zisk,gpu
    ```

    Besides the binary, the build writes the guest program's verification key to `crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk`. The deployer reads it from there when it initializes the L1 contracts, so deploy from the same checkout you built the prover from, or pass the file with `--zisk-vk-path`.

- An ethrex L2 deployed with ZisK proofs required. See [Deploying with ZisK proofs](#deploying-with-zisk-proofs) below, since there is one step more than for the other provers.

## Deploying with ZisK proofs

ZisK verifies its proofs on-chain with `ZiskVerifier.sol`, a snarkJS PLONK verifier whose verification key and recursion root are generated per ZisK release. The contract is GPL licensed, so ethrex does not embed it in the deployer the way it does for SP1: deploy it yourself from the ZisK release the prover is pinned to and hand its address to the deployer. From `crates/l2`:

```shell
make deploy-zisk-verifier
```

This fetches `IZiskVerifier.sol`, `PlonkVerifier.sol` and `ZiskVerifier.sol` from the ZisK v1.3.2-alpha tag, compiles them with `solc` and deploys the verifier with `rex` using the Makefile's L1 RPC URL and deployer key, printing the address. Then deploy the L1 contracts requiring ZisK proofs:

```shell
export ETHREX_DEPLOYER_ZISK_VERIFIER_ADDRESS=<address printed above>
make deploy-l1-zisk
```

Outside the Makefile, the equivalent flags of `ethrex l2 deploy` are `--zisk true` and `--zisk.verifier-address <address>`. The deployer refuses to run with `--zisk true` and no verifier address.

> [!IMPORTANT]
> The verifier, the proving keys and the guest program must all come from the same ZisK release. A proof wrapped with another release's PLONK key verifies off-chain but reverts on-chain with `InvalidZiskProof`.

## Start an ethrex L2 ZisK prover

Once you have your ethrex L2 deployed with ZisK enabled, you can start the ZisK prover using the following command:

```shell
ethrex l2 prover \
--backend zisk \
--proof-coordinators http://localhost:3900
```

> [!IMPORTANT]
> Regardless of the installation method used for ethrex, make sure the binary you are using has ZisK support, and also GPU support if you intend to run a ZisK GPU prover.

> [!NOTE]
> The flag `--proof-coordinators` is used to specify one or more proof coordinator URLs. This is so because the prover is capable of proving ethrex L2 batches from multiple sequencers. We are particularly setting it to `localhost:3900` because the command above uses the port `3900` for the proof coordinator by default (to learn more about the proof coordinator, read the ethrex L2 sequencer and ethrex L2 prover sections).
> We choose ZisK as the backend to indicate the prover to generate ZisK proofs.

On startup the prover builds the in-process ZisK client, runs the program setup for the embedded guest (cached under `~/.zisk/cache`, so only the first start pays for it) and logs the program verification key. That key must match the one registered on-chain for the commit hash the sequencer runs; the deployer registers the one read at deployment and `upgradeZiskVerificationKey` changes it.

## How ZisK proofs are verified

ZisK exposes 64 32-bit public output slots, while the L2 batch output the OnChainProposer reconstructs is longer than that. The ZisK guest therefore commits `sha256` of the encoded batch output, and the OnChainProposer hashes the public inputs it rebuilds from the batch commitment the same way before calling the verifier. The proof sender submits only the PLONK proof bytes; the program key comes from the contract's storage and the recursion root from the verifier contract.

## Troubleshooting

### The prover exits with `failed to build the ZisK prover client`

The ZisK bundle is missing or incomplete. Check that `~/.zisk/provingKey` exists (or point `ZISK_HOME` at the bundle), that the PLONK key is installed under `~/.zisk/provingKeySnark`, and, for a GPU prover, that `cargo-zisk --version` reports `[gpu]`. If the message ends in `Not enough GPU memory to run the proof`, the card is too small for this ZisK release's GPU plan (see the prerequisites); rebuild ethrex without the `gpu` feature to prove on CPU.

### Proving fails at the PLONK wrap with `cannot enable executable stack as shared object requires`

The host's glibc (2.41 or newer) refuses to load ZisK's `final.so`. Restart the prover with `GLIBC_TUNABLES=glibc.rtld.execstack=2` in its environment. The STARK phases before the wrap are unaffected, which is why the failure only shows up after a few minutes of proving.

### Proofs are rejected on-chain with `InvalidZiskProof`

The proof sender deletes the proof and a prover regenerates it, so a persistent rejection points to a configuration mismatch: the verifier contract was deployed from a ZisK release other than v1.3.2-alpha, the proving keys belong to another release, or the verification key registered on-chain is not the one of the guest program the prover embeds. Compare the key the prover logs at startup with `verificationKeys(commitHash, 3)` on the OnChainProposer.

### Building ethrex fails while compiling `proofman-starks-lib-c`

The ZisK prover library needs the system packages from ZisK's installation guide (`libgmp-dev`, `libomp-dev`, `nlohmann-json3-dev`, `nasm`, `libopenmpi-dev`, `libsodium-dev`, `libsecp256k1-dev`, among others). On a host with a CUDA toolkit the build also compiles the CUDA kernels; set `CUDA_ARCHS` to your GPU's compute capability (for example `CUDA_ARCHS=86` for an RTX 3090) to shorten it.
